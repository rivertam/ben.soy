//! The margin instruments of the planes post: small server-drawn figures in
//! the ice figure's grammar — hairline grids, mono labels, one fact each,
//! drawn to scale. The jet fuel as 55-gallon drums, this ticket's seat on
//! the aircraft's cabin plan, and the same CO₂e quoted at the wheel — in
//! whichever of coast-to-coast runs, commute months, or Hummer tanks fits
//! its size — stack with the ice callout in `.instrument-rail` (see
//! `charts_section`). The route's great circle on its graticule rides the
//! top of the margin column instead (the `.desk-route` card in `mod.rs`).

use topcoat::{
    Result,
    view::{component, view},
};

use super::{
    airports::Airport,
    emissions::{Cabin, cabin_weight},
    format::format_whole,
    sources::cite,
};

/// A 55-US-gallon drum in litres — the standard barrel the fuel figure
/// counts in. Definitional (55 gal × 3.785 L), not an emissions claim.
const DRUM_LITRES: f64 = 208.2;

/// An f64 clipped for an SVG attribute: two decimals, no float-representation
/// noise in the served markup.
fn f2(v: f64) -> String {
    format!("{v:.2}")
}

// ---------- great-circle math (route figure) ----------

/// Points along the great circle between two coordinates, as (lat, lon)
/// degrees with longitudes *unwrapped*: each successive longitude is shifted
/// by ±360° to stay within 180° of its predecessor, so a route crossing the
/// antimeridian stays one continuous polyline instead of splitting.
fn great_circle_points(lat1: f64, lon1: f64, lat2: f64, lon2: f64, n: usize) -> Vec<(f64, f64)> {
    let to_xyz = |lat: f64, lon: f64| {
        let (la, lo) = (lat.to_radians(), lon.to_radians());
        (la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin())
    };
    let a = to_xyz(lat1, lon1);
    let b = to_xyz(lat2, lon2);
    let dot = (a.0 * b.0 + a.1 * b.1 + a.2 * b.2).clamp(-1.0, 1.0);
    let omega = dot.acos();

    let mut points = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let (lat, lon) = if omega.sin() < 1e-9 {
            // Coincident (or pathologically antipodal) endpoints: fall back
            // to linear interpolation, which is exact for the former.
            (lat1 + (lat2 - lat1) * t, lon1 + (lon2 - lon1) * t)
        } else {
            let (wa, wb) = (
                ((1.0 - t) * omega).sin() / omega.sin(),
                (t * omega).sin() / omega.sin(),
            );
            let p = (
                wa * a.0 + wb * b.0,
                wa * a.1 + wb * b.1,
                wa * a.2 + wb * b.2,
            );
            (
                p.2.clamp(-1.0, 1.0).asin().to_degrees(),
                p.1.atan2(p.0).to_degrees(),
            )
        };
        points.push((lat, lon));
    }

    // Unwrap longitudes into one continuous run.
    for i in 1..points.len() {
        let prev = points[i - 1].1;
        while points[i].1 - prev > 180.0 {
            points[i].1 -= 360.0;
        }
        while points[i].1 - prev < -180.0 {
            points[i].1 += 360.0;
        }
    }
    points
}

/// The great circles of a multi-stop route as one continuous polyline:
/// per-leg sampled points concatenated with the longitude unwrap carried
/// across leg joins (each leg's `great_circle_points` run starts from raw
/// coordinates, so a join can otherwise jump by ±360°). Returns the points
/// and, for each stop, its index into them.
fn route_polyline(stops: &[(f64, f64)], n_per_leg: usize) -> (Vec<(f64, f64)>, Vec<usize>) {
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut stop_at: Vec<usize> = Vec::new();
    for w in stops.windows(2) {
        let mut seg = great_circle_points(w[0].0, w[0].1, w[1].0, w[1].1, n_per_leg);
        if pts.is_empty() {
            stop_at.push(0);
            pts.extend(seg);
        } else {
            // The leg starts where the previous one ended; its raw start
            // longitude equals the previous unwrapped end modulo 360°.
            let shift = ((pts.last().expect("points").1 - seg[0].1) / 360.0).round() * 360.0;
            for p in &mut seg {
                p.1 += shift;
            }
            pts.extend(seg.into_iter().skip(1));
        }
        stop_at.push(pts.len() - 1);
    }
    if pts.is_empty() {
        // A single-stop (or empty) route: plot the point itself.
        for &(lat, lon) in stops {
            stop_at.push(pts.len());
            pts.push((lat, lon));
        }
    }
    (pts, stop_at)
}

/// The graticule spacing for a span of degrees: the smallest conventional
/// step that keeps the line count readable (≤6 intervals).
fn graticule_step(span_deg: f64) -> f64 {
    for step in [5.0, 10.0, 15.0, 20.0, 30.0, 45.0] {
        if span_deg / step <= 6.0 {
            return step;
        }
    }
    60.0
}

/// The route figure's projected window: viewBox origin/size, the x scale,
/// the graticule step, and the latitude/longitude bounds the graticule
/// draws over. Extracted from the component so the geometry is testable.
struct RouteWindow {
    x0: f64,
    y0: f64,
    vw: f64,
    vh: f64,
    xs: f64,
    step: f64,
    lat_lo: f64,
    lat_hi: f64,
    lon_lo: f64,
    lon_hi: f64,
}

fn route_window(pts: &[(f64, f64)]) -> RouteWindow {
    let lat_min = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let lat_max = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let lon_min = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let lon_max = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let mid_lat = ((lat_min + lat_max) / 2.0).to_radians();
    let xs = mid_lat.cos().max(0.2);

    // Pad the box; keep a minimum window so short hops don't over-zoom —
    // but the map never extends past the poles.
    let lat_pad = ((lat_max - lat_min) * 0.22).max(2.5);
    let lon_pad = ((lon_max - lon_min) * 0.12).max(2.5 / xs);
    let (lat_lo, lat_hi) = (
        (lat_min - lat_pad).max(-90.0),
        (lat_max + lat_pad).min(90.0),
    );
    let (mut lon_lo, mut lon_hi) = (lon_min - lon_pad, lon_max + lon_pad);

    // Cap the aspect: a near-meridional route (JFK→SCL) would otherwise
    // tower thirteen-to-one in the rail; widen its window instead.
    let vh = lat_hi - lat_lo;
    let vw = (lon_hi - lon_lo) * xs;
    let min_vw = vh / 1.15;
    if vw < min_vw {
        let extra = (min_vw - vw) / (2.0 * xs);
        lon_lo -= extra;
        lon_hi += extra;
    }
    let vw = (lon_hi - lon_lo) * xs;

    // The graticule step follows the final window, not the bare route.
    let step = graticule_step((lat_hi - lat_lo).max(lon_hi - lon_lo));

    RouteWindow {
        x0: lon_lo * xs,
        y0: -lat_hi,
        vw,
        vh,
        xs,
        step,
        lat_lo,
        lat_hi,
        lon_lo,
        lon_hi,
    }
}

/// One of the route's stops, projected for the figure: the dot position and
/// the label beside it, pre-formatted for SVG attributes.
struct StopMark {
    iata: String,
    x: f64,
    y: f64,
}

/// The route as its great circle(s) on a local equirectangular plot —
/// x is longitude scaled by cos(mid-latitude) so shapes stay honest, the
/// graticule is the scale, and the dashed oxide line is the same grammar as
/// the charts' flight tick. `stops` is the whole itinerary in order —
/// origin, any layovers, destination.
#[component]
pub async fn route_figure(stops: Vec<Airport>, round_trip: bool, km_flown: String) -> Result {
    let coords: Vec<(f64, f64)> = stops.iter().map(|a| (a.lat, a.lon)).collect();
    let (pts, stop_at) = route_polyline(&coords, 64);
    let RouteWindow {
        x0,
        y0,
        vw,
        vh,
        xs,
        step,
        lat_lo,
        lat_hi,
        lon_lo,
        lon_hi,
    } = route_window(&pts);
    let trip_label = if round_trip { "round trip" } else { "one way" };
    let leg_count = stops.len().saturating_sub(1).max(1);

    let path: String = pts
        .iter()
        .map(|(lat, lon)| format!("{:.2},{:.2}", lon * xs, -lat))
        .collect::<Vec<_>>()
        .join(" ");

    let meridians: Vec<f64> = {
        let mut m = (lon_lo / step).ceil() * step;
        let mut v = Vec::new();
        while m <= lon_hi {
            v.push(m);
            m += step;
        }
        v
    };
    let parallels: Vec<f64> = {
        let mut p = (lat_lo / step).ceil() * step;
        let mut v = Vec::new();
        while p <= lat_hi {
            v.push(p);
            p += step;
        }
        v
    };

    let sw = vw * 0.004;
    let fs = vw * 0.062;
    let dot = vw * 0.014;
    // Stop positions come from the *unwrapped* path, not the raw
    // coordinates — on an antimeridian-crossing route a raw longitude can
    // sit a world away from the polyline's actual pass through the stop.
    let marks: Vec<StopMark> = stops
        .iter()
        .zip(&stop_at)
        .map(|(a, &i)| StopMark {
            iata: a.iata.clone(),
            x: pts[i].1 * xs,
            y: -pts[i].0,
        })
        .collect();
    let (fx, fy) = (marks[0].x, marks[0].y);
    let (tx, ty) = {
        let last = marks.last().expect("route stops");
        (last.x, last.y)
    };
    let via_marks: Vec<&StopMark> = marks
        .get(1..marks.len().saturating_sub(1))
        .unwrap_or_default()
        .iter()
        .collect();
    // Labels sit below their dots, nudged inside the box; when the dot is
    // too close to the bottom edge the label flips above it instead.
    let label_x = |x: f64| x.clamp(x0 + fs * 1.6, x0 + vw - fs * 1.6);
    let label_y = |y: f64| {
        if y + fs * 1.45 > y0 + vh - fs * 0.35 {
            y - fs * 0.9
        } else {
            y + fs * 1.45
        }
    };
    let via_names = via_marks
        .iter()
        .map(|m| m.iata.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let aria = if via_marks.is_empty() {
        format!(
            "The great-circle route from {} to {} on a {}-degree graticule, {}",
            marks[0].iata,
            marks[marks.len() - 1].iata,
            step,
            trip_label
        )
    } else {
        format!(
            "The great-circle route from {} via {} to {} on a {}-degree graticule, {}",
            marks[0].iata,
            via_names,
            marks[marks.len() - 1].iata,
            step,
            trip_label
        )
    };
    let fig_note = if leg_count == 1 {
        format!("the great circle · {trip_label} · {step}° graticule")
    } else {
        format!("{leg_count} great circles · {trip_label} · {step}° graticule")
    };
    // Via labels stagger above/below the line and drop out rather than
    // pile up: a label only lands where it won't collide with one already
    // placed (the endpoints always win). Suppressed stops keep their dots,
    // and the aria label names every stop regardless.
    let mut placed: Vec<(f64, f64)> = vec![(label_x(fx), label_y(fy)), (label_x(tx), label_y(ty))];
    let via_labels: Vec<StopMark> = via_marks
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let x = label_x(m.x);
            let above = m.y - fs * 0.9;
            let y = if i % 2 == 1 && above > y0 + fs * 1.2 {
                above
            } else {
                label_y(m.y)
            };
            let clash = placed
                .iter()
                .any(|&(px, py)| (px - x).abs() < fs * 2.2 && (py - y).abs() < fs * 1.05);
            if clash {
                return None;
            }
            placed.push((x, y));
            Some(StopMark {
                iata: m.iata.clone(),
                x,
                y,
            })
        })
        .collect();
    let via_dots: Vec<(String, String)> = via_marks.iter().map(|m| (f2(m.x), f2(m.y))).collect();
    let from_iata = marks[0].iata.clone();
    let to_iata = marks[marks.len() - 1].iata.clone();

    view! {
        <aside class="instrument" aria-label="Route flown">
            <p>
                "Route flown: "
                <strong>(km_flown.as_str())</strong>
            </p>
            <svg
                viewBox=(format!("{x0:.2} {y0:.2} {vw:.2} {vh:.2}"))
                role="img"
                aria-label=(aria)
            >
                <g stroke="var(--hairline)" stroke-width=(f2(sw))>
                    for m in meridians {
                        <line x1=(f2(m * xs)) y1=(f2(y0)) x2=(f2(m * xs)) y2=(f2(y0 + vh)) />
                    }
                    for p in parallels {
                        <line x1=(f2(x0)) y1=(f2(-p)) x2=(f2(x0 + vw)) y2=(f2(-p)) />
                    }
                </g>
                <polyline
                    points=(path)
                    fill="none"
                    stroke="var(--cost)"
                    stroke-width=(f2(vw * 0.011))
                    stroke-dasharray=(format!("{:.2} {:.2}", vw * 0.022, vw * 0.016))
                    stroke-linecap="round"
                />
                <circle
                    cx=(f2(fx))
                    cy=(f2(fy))
                    r=(f2(dot))
                    fill="var(--card)"
                    stroke="var(--cost)"
                    stroke-width=(f2(sw * 2.5))
                />
                <circle cx=(f2(tx)) cy=(f2(ty)) r=(f2(dot)) fill="var(--cost)" />
                for d in via_dots {
                    <circle
                        cx=(d.0)
                        cy=(d.1)
                        r=(f2(dot * 0.72))
                        fill="var(--cost)"
                        stroke="var(--card)"
                        stroke-width=(f2(sw * 2.0))
                    />
                }
                for l in via_labels {
                    <text
                        class="fig-mono"
                        x=(f2(l.x))
                        y=(f2(l.y))
                        text-anchor="middle"
                        font-size=(f2(fs * 0.85))
                        fill="var(--ink-2)"
                        stroke="var(--card)"
                        stroke-width=(f2(fs * 0.3))
                        paint-order="stroke"
                    >(l.iata)</text>
                }
                <text
                    class="fig-mono"
                    x=(f2(label_x(fx)))
                    y=(f2(label_y(fy)))
                    text-anchor="middle"
                    font-size=(f2(fs))
                    fill="var(--ink-2)"
                    stroke="var(--card)"
                    stroke-width=(f2(fs * 0.3))
                    paint-order="stroke"
                >(from_iata.as_str())</text>
                <text
                    class="fig-mono"
                    x=(f2(label_x(tx)))
                    y=(f2(label_y(ty)))
                    text-anchor="middle"
                    font-size=(f2(fs))
                    fill="var(--ink-2)"
                    stroke="var(--card)"
                    stroke-width=(f2(fs * 0.3))
                    paint-order="stroke"
                >(to_iata.as_str())</text>
            </svg>
            <div class="fig-note">
                (fig_note)
            </div>
        </aside>
    }
}

// ---------- tally grammar (fuel + driving figures) ----------

/// "≈1.8" / "≈18" — one decimal under ten units, whole above, never "0.0".
fn format_tally(units: f64) -> String {
    if units >= 9.95 {
        format_whole(units)
    } else if units >= 0.095 {
        let s = format!("{units:.1}");
        s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
    } else {
        format!("{units:.2}")
    }
}

/// Per-glyph fill fractions for a tally of `count` units: full glyphs, then
/// the remainder filled to its fraction. Never empty — a count of zero draws
/// one empty glyph.
fn tally_fracs(count: f64) -> Vec<f64> {
    let slots = count.ceil().max(1.0) as usize;
    (0..slots)
        .map(|i| (count - i as f64).clamp(0.0, 1.0))
        .collect()
}

// ---------- fuel figure ----------

/// The ticket's share of the jet fuel, counted out in 55-gallon drums —
/// full drums filled, the remainder filled to its fraction.
#[component]
pub async fn fuel_figure(litres: f64, litres_label: String) -> Result {
    let drums = litres / DRUM_LITRES;
    let fracs = tally_fracs(drums);
    let slots = fracs.len();
    let cols = slots.min(8);
    let rows = slots.div_ceil(8);

    // Drum glyph geometry, in viewBox units.
    let (dw, dh, gap, m) = (1.0, 1.35, 0.3, 0.12);
    let vw = cols as f64 * (dw + gap) - gap + 2.0 * m;
    let vh = rows as f64 * (dh + gap) - gap + 2.0 * m;
    let inset = 0.09;

    let fills: Vec<(f64, f64, f64)> = fracs
        .iter()
        .enumerate()
        .map(|(i, &frac)| {
            let x = m + (i % 8) as f64 * (dw + gap);
            let y = m + (i / 8) as f64 * (dh + gap);
            (x, y, frac)
        })
        .collect();

    view! {
        <aside class="instrument" aria-label="Jet fuel burned">
            <p>
                "Jet fuel burned: "
                <strong>(litres_label.as_str())</strong>
                " "
                cite(id: "myclimate")
                cite(id: "jetfuel-density")
            </p>
            <svg
                viewBox=(format!("0 0 {vw:.2} {vh:.2}"))
                role="img"
                aria-label=(format!(
                    "{} litres of jet fuel drawn as {} fifty-five-gallon drums",
                    format_whole(litres),
                    format_tally(drums)
                ))
                style=(format!("max-width:{:.2}rem", cols as f64 * 2.1))
            >
                for (x, y, frac) in fills {
                    <g>
                        <rect
                            x=(f2(x + inset))
                            y=(f2(y + inset + (dh - 2.0 * inset) * (1.0 - frac)))
                            width=(f2(dw - 2.0 * inset))
                            height=(f2((dh - 2.0 * inset) * frac))
                            fill="var(--cost)"
                            fill-opacity="0.3"
                        />
                        <rect
                            x=(f2(x))
                            y=(f2(y))
                            width=(f2(dw))
                            height=(f2(dh))
                            rx="0.07"
                            fill="none"
                            stroke="var(--muted)"
                            stroke-width="0.045"
                        />
                        <line
                            x1=(f2(x))
                            y1=(f2(y + dh * 0.33))
                            x2=(f2(x + dw))
                            y2=(f2(y + dh * 0.33))
                            stroke="var(--muted)"
                            stroke-width="0.03"
                        />
                        <line
                            x1=(f2(x))
                            y1=(f2(y + dh * 0.67))
                            x2=(f2(x + dw))
                            y2=(f2(y + dh * 0.67))
                            stroke="var(--muted)"
                            stroke-width="0.03"
                        />
                    </g>
                }
            </svg>
            <div class="fig-note">
                (format!(
                    "≈{} drums of 55 gal (208 L) — this ticket’s share of the tanks",
                    format_tally(drums)
                ))
            </div>
        </aside>
    }
}

// ---------- driving figure ----------

/// EPA's per-gallon figure: 8,887 g of CO₂ from burning one gallon of
/// gasoline — the same page the ≈400 g/mi typical-vehicle number comes from.
const KG_PER_GALLON: f64 = 8.887;
/// New York → Los Angeles by road; routing services quote ≈2,790 mi on
/// I-80. Definitional, not an emissions claim.
const COAST_MILES: f64 = 2790.0;
/// A ≈35-mpg compact — the 2025 Corolla's EPA combined rating.
const COMPACT_MPG: f64 = 35.0;
/// A ≈20-mpg sports car — 911 Carrera 21, Corvette 19, Mustang GT 19.
const SPORTS_MPG: f64 = 20.0;
/// The commute: 15 miles each way, so 30 driven miles a commuting day.
const COMMUTE_MILES_PER_DAY: f64 = 30.0;
/// Commuting days in a month: five a week, 52 weeks over 12 months.
const COMMUTE_DAYS_PER_MONTH: f64 = 5.0 * 52.0 / 12.0;
/// The Hummer H2's tank, in gallons — 32 across every model year.
const HUMMER_TANK_GAL: f64 = 32.0;

/// One coast-to-coast run in the compact, ≈708 kg.
const COAST_RUN_KG: f64 = COAST_MILES * KG_PER_GALLON / COMPACT_MPG;
/// One month of the commute in the sports car, ≈289 kg.
const COMMUTE_MONTH_KG: f64 =
    COMMUTE_MILES_PER_DAY * KG_PER_GALLON / SPORTS_MPG * COMMUTE_DAYS_PER_MONTH;
/// One Hummer tank, filled and burned, ≈284 kg.
const HUMMER_TANK_KG: f64 = HUMMER_TANK_GAL * KG_PER_GALLON;

/// Which unit showcases the flight at the wheel — the largest one it
/// clears three of, so the tally always lands with weight: coast-to-coast
/// runs first, commute months next, Hummer tank fills for anything smaller.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RoadUnit {
    CoastRuns,
    CommuteMonths,
    HummerTanks,
}

fn road_unit(flight_kg: f64) -> RoadUnit {
    if flight_kg / COAST_RUN_KG > 3.0 {
        RoadUnit::CoastRuns
    } else if flight_kg / COMMUTE_MONTH_KG > 3.0 {
        RoadUnit::CommuteMonths
    } else {
        RoadUnit::HummerTanks
    }
}

/// The flight's CO₂e quoted once at the wheel, in the unit [`road_unit`]
/// picks for its size — one tally in the fuel figure's grammar, the
/// remainder drawn to its fraction.
#[component]
pub async fn driving_figure(flight_kg: f64) -> Result {
    let unit = road_unit(flight_kg);
    let coast_pick = unit == RoadUnit::CoastRuns;
    let commute_pick = unit == RoadUnit::CommuteMonths;
    let tanks_pick = unit == RoadUnit::HummerTanks;

    // Coast-to-coast runs: one road chip per crossing, the oxide line
    // driven as far as this flight's carbon takes it.
    let runs = flight_kg / COAST_RUN_KG;
    let run_fracs = tally_fracs(runs);
    let r_cols = run_fracs.len().min(4);
    let (cw, ch, cgap, cm) = (3.0, 0.95, 0.3, 0.06);
    let r_vw = r_cols as f64 * (cw + cgap) - cgap + 2.0 * cm;
    let r_vh = run_fracs.len().div_ceil(4) as f64 * (ch + cgap) - cgap + 2.0 * cm;
    let chips: Vec<(f64, f64, f64)> = run_fracs
        .iter()
        .enumerate()
        .map(|(i, &frac)| {
            (
                cm + (i % 4) as f64 * (cw + cgap),
                cm + (i / 4) as f64 * (ch + cgap),
                frac,
            )
        })
        .collect();

    // Commute months: each strip a year of the commute, month ticks along
    // it, filled to how deep into it this flight reaches.
    let months = flight_kg / COMMUTE_MONTH_KG;
    let month_ticks: Vec<f64> = (1..12).map(|q| q as f64 / 12.0).collect();
    let strip_fracs = tally_fracs(months / 12.0);
    let (sw, sh, sgap, sm) = (13.2, 0.62, 0.26, 0.06);
    let s_vw = sw + 2.0 * sm;
    let s_vh = strip_fracs.len() as f64 * (sh + sgap) - sgap + 2.0 * sm;
    let strips: Vec<(f64, f64)> = strip_fracs
        .iter()
        .enumerate()
        .map(|(i, &frac)| (sm + i as f64 * (sh + sgap), frac))
        .collect();

    // Hummer tanks: one jerrycan per 32-gallon fill, drum-style.
    let tanks = flight_kg / HUMMER_TANK_KG;
    let tank_fracs = tally_fracs(tanks);
    let t_cols = tank_fracs.len().min(8);
    let (tw, th, cap_h, tgap, tm) = (1.05, 1.3, 0.16, 0.3, 0.1);
    let t_vw = t_cols as f64 * (tw + tgap) - tgap + 2.0 * tm;
    let t_vh = tank_fracs.len().div_ceil(8) as f64 * (th + tgap) - tgap + 2.0 * tm;
    let tank_glyphs: Vec<(f64, f64, f64)> = tank_fracs
        .iter()
        .enumerate()
        .map(|(i, &frac)| {
            (
                tm + (i % 8) as f64 * (tw + tgap),
                tm + (i / 8) as f64 * (th + tgap),
                frac,
            )
        })
        .collect();

    view! {
        <aside class="instrument" aria-label="The same CO₂e, by road">
            <p>
                "The same CO₂e, by road "
                cite(id: "cars")
                if coast_pick {
                    cite(id: "compact-mpg")
                }
                if commute_pick {
                    cite(id: "sports-mpg")
                }
                if tanks_pick {
                    cite(id: "hummer")
                }
            </p>
            if coast_pick {
            <svg
                viewBox=(format!("0 0 {r_vw:.2} {r_vh:.2}"))
                role="img"
                aria-label=(format!(
                    "{} one-way New York to Los Angeles drives in a 35-mpg compact car, drawn as road chips",
                    format_tally(runs)
                ))
                style=(format!("max-width:{:.2}rem", r_cols as f64 * 3.45))
            >
                for (x, y, frac) in chips {
                    <g>
                        <rect
                            x=(f2(x))
                            y=(f2(y))
                            width=(f2(cw))
                            height=(f2(ch))
                            rx="0.1"
                            fill="none"
                            stroke="var(--muted)"
                            stroke-width="0.045"
                        />
                        <line
                            x1=(f2(x + 0.45))
                            y1=(f2(y + ch / 2.0))
                            x2=(f2(x + cw - 0.45))
                            y2=(f2(y + ch / 2.0))
                            stroke="var(--hairline)"
                            stroke-width="0.09"
                            stroke-linecap="round"
                        />
                        if frac > 0.005 {
                            <line
                                x1=(f2(x + 0.45))
                                y1=(f2(y + ch / 2.0))
                                x2=(f2(x + 0.45 + (cw - 0.9) * frac))
                                y2=(f2(y + ch / 2.0))
                                stroke="var(--cost)"
                                stroke-width="0.11"
                                stroke-linecap="round"
                            />
                        }
                        <circle
                            cx=(f2(x + 0.45))
                            cy=(f2(y + ch / 2.0))
                            r="0.08"
                            fill="var(--muted)"
                        />
                        if frac >= 1.0 {
                            <circle
                                cx=(f2(x + cw - 0.45))
                                cy=(f2(y + ch / 2.0))
                                r="0.11"
                                fill="var(--cost)"
                            />
                        } else {
                            <circle
                                cx=(f2(x + cw - 0.45))
                                cy=(f2(y + ch / 2.0))
                                r="0.11"
                                fill="var(--card)"
                                stroke="var(--muted)"
                                stroke-width="0.045"
                            />
                        }
                    </g>
                }
            </svg>
            <div class="fig-note">
                <strong>(format!("≈{}×", format_tally(runs)))</strong>
                " New York → L.A. in a compact car"
            </div>
            <div class="fig-note fig-mono">
                "NY → L.A. ≈2,790 mi · compact ≈35 mpg · gas 8.9 kg CO₂/gal"
            </div>
            }
            if commute_pick {
            <svg
                viewBox=(format!("0 0 {s_vw:.2} {s_vh:.2}"))
                role="img"
                aria-label=(format!(
                    "{} months of commuting 15 miles each way, five days a week, in a 20-mpg sports car — drawn on a year strip",
                    format_tally(months)
                ))
                style="max-width:13.5rem"
            >
                for (y, frac) in strips {
                    <g>
                        <rect
                            x=(f2(sm + 0.07))
                            y=(f2(y + 0.07))
                            width=(f2((sw - 0.14) * frac))
                            height=(f2(sh - 0.14))
                            fill="var(--cost)"
                            fill-opacity="0.3"
                        />
                        <rect
                            x=(f2(sm))
                            y=(f2(y))
                            width=(f2(sw))
                            height=(f2(sh))
                            rx="0.08"
                            fill="none"
                            stroke="var(--muted)"
                            stroke-width="0.045"
                        />
                        for t in month_ticks.iter().copied() {
                            <line
                                x1=(f2(sm + sw * t))
                                y1=(f2(y))
                                x2=(f2(sm + sw * t))
                                y2=(f2(y + sh))
                                stroke="var(--muted)"
                                stroke-width="0.03"
                            />
                        }
                    </g>
                }
            </svg>
            <div class="fig-note">
                <strong>(format!("≈{}", format_tally(months)))</strong>
                " months of commuting 15 miles each way, 5 days a week, in a sports car \
                 — the strip a year"
            </div>
            <div class="fig-note fig-mono">
                "sports ≈20 mpg · gas 8.9 kg CO₂/gal"
            </div>
            }
            if tanks_pick {
            <svg
                viewBox=(format!("0 0 {t_vw:.2} {t_vh:.2}"))
                role="img"
                aria-label=(format!(
                    "{} fill-ups of a Hummer H2's 32-gallon tank, drawn as fuel cans",
                    format_tally(tanks)
                ))
                style=(format!("max-width:{:.2}rem", t_cols as f64 * 1.9))
            >
                for (x, y, frac) in tank_glyphs {
                    <g>
                        <rect
                            x=(f2(x + 0.08))
                            y=(f2(y + cap_h + 0.08 + (th - cap_h - 0.16) * (1.0 - frac)))
                            width=(f2(tw - 0.16))
                            height=(f2((th - cap_h - 0.16) * frac))
                            fill="var(--cost)"
                            fill-opacity="0.3"
                        />
                        <rect
                            x=(f2(x + tw * 0.58))
                            y=(f2(y))
                            width="0.26"
                            height=(f2(cap_h + 0.05))
                            rx="0.03"
                            fill="none"
                            stroke="var(--muted)"
                            stroke-width="0.045"
                        />
                        <rect
                            x=(f2(x))
                            y=(f2(y + cap_h))
                            width=(f2(tw))
                            height=(f2(th - cap_h))
                            rx="0.09"
                            fill="none"
                            stroke="var(--muted)"
                            stroke-width="0.045"
                        />
                        <line
                            x1=(f2(x + 0.18))
                            y1=(f2(y + cap_h + 0.18))
                            x2=(f2(x + tw - 0.18))
                            y2=(f2(y + th - 0.18))
                            stroke="var(--muted)"
                            stroke-width="0.03"
                        />
                        <line
                            x1=(f2(x + tw - 0.18))
                            y1=(f2(y + cap_h + 0.18))
                            x2=(f2(x + 0.18))
                            y2=(f2(y + th - 0.18))
                            stroke="var(--muted)"
                            stroke-width="0.03"
                        />
                    </g>
                }
            </svg>
            <div class="fig-note">
                <strong>(format!("≈{}", format_tally(tanks)))</strong>
                " Hummer H2 tanks, filled and burned"
            </div>
            <div class="fig-note fig-mono">
                "H2 tank 32 gal · gas 8.9 kg CO₂/gal"
            </div>
            }
        </aside>
    }
}

// ---------- seat figure ----------

/// Seat-dot radius, in economy-seat-pitch units.
const SEAT_R: f64 = 0.34;
/// Clear width of an aisle beyond the regular seat spacing.
const AISLE: f64 = 0.7;
/// The galley/bulkhead gap between cabin sections.
const GALLEY: f64 = 0.7;

/// One cabin section of the plan: which class sits there, its seat blocks
/// across the fuselage (window to window), and its column count fore-to-aft.
/// The last column may narrow (`tail_blocks`) where the fuselage tapers.
struct SeatSection {
    cabin: Cabin,
    blocks: &'static [usize],
    tail_blocks: Option<&'static [usize]>,
    cols: usize,
}

/// The cabin mix drawn per haul model, solved so the seats' myclimate
/// weights sum back to the model's average seat count (within 1% — the
/// tests hold this): long-haul 8F/54J/222Y against 280.39 seats, short-haul
/// 20J/138Y against 158.44. A short-haul first ticket gets the domestic
/// variant — a small first cabin up front instead of the business rows.
fn seat_sections(long_haul: bool, cabin: Cabin) -> Vec<SeatSection> {
    if long_haul {
        vec![
            SeatSection {
                cabin: Cabin::First,
                blocks: &[1, 2, 1],
                tail_blocks: None,
                cols: 2,
            },
            SeatSection {
                cabin: Cabin::Business,
                blocks: &[2, 2, 2],
                tail_blocks: None,
                cols: 9,
            },
            SeatSection {
                cabin: Cabin::Economy,
                blocks: &[3, 3, 3],
                tail_blocks: Some(&[2, 2, 2]),
                cols: 25,
            },
        ]
    } else if cabin == Cabin::First {
        vec![
            SeatSection {
                cabin: Cabin::First,
                blocks: &[2, 2],
                tail_blocks: None,
                cols: 2,
            },
            SeatSection {
                cabin: Cabin::Economy,
                blocks: &[3, 3],
                tail_blocks: None,
                cols: 24,
            },
        ]
    } else {
        vec![
            SeatSection {
                cabin: Cabin::Business,
                blocks: &[2, 2],
                tail_blocks: None,
                cols: 5,
            },
            SeatSection {
                cabin: Cabin::Economy,
                blocks: &[3, 3],
                tail_blocks: None,
                cols: 23,
            },
        ]
    }
}

/// Seat-centre offsets across the fuselage for one column, centred on the
/// aisle line: `scale` between neighbours in a block, `scale + AISLE`
/// across an aisle.
fn column_ys(blocks: &[usize], scale: f64) -> Vec<f64> {
    let n: usize = blocks.iter().sum();
    let span = (n - 1) as f64 * scale + (blocks.len() - 1) as f64 * AISLE;
    let mut ys = Vec::with_capacity(n);
    let mut y = -span / 2.0;
    for &b in blocks {
        for _ in 0..b {
            ys.push(y);
            y += scale;
        }
        y += AISLE;
    }
    ys
}

struct SeatDot {
    x: f64,
    y: f64,
    r: f64,
    mine: bool,
}

struct SeatMap {
    dots: Vec<SeatDot>,
    /// Cabin bounds along the fuselage: aft of the nose, fore of the tail.
    nose: f64,
    body_end: f64,
    /// Tail-cone tip — the fuselage's full length (the nose tip is x = 0).
    tip: f64,
    /// Fuselage half-width.
    half: f64,
}

/// The aircraft plan, nose left: every seat placed, sized so its *floor
/// area* is proportional to its cabin's myclimate weight — first and
/// business draw bigger because their share of the bill is bigger. Your
/// seat sits mid-cabin in your class.
fn seat_map(long_haul: bool, cabin: Cabin) -> SeatMap {
    let sections = seat_sections(long_haul, cabin);
    let w_econ = cabin_weight(long_haul, Cabin::Economy);
    let (nose, tail) = if long_haul { (4.6, 4.6) } else { (3.8, 3.9) };

    let mut dots = Vec::new();
    let mut half_span = 0.0_f64;
    let mut x = nose;
    for s in &sections {
        let w = cabin_weight(long_haul, s.cabin);
        let scale = (w / w_econ).sqrt();
        let mine_col = if s.cabin == cabin {
            Some(s.cols / 2)
        } else {
            None
        };
        for col in 0..s.cols {
            let blocks = if col + 1 == s.cols {
                s.tail_blocks.unwrap_or(s.blocks)
            } else {
                s.blocks
            };
            let cx = x + (col as f64 + 0.5) * scale;
            let ys = column_ys(blocks, scale);
            half_span = half_span.max(-ys[0] + SEAT_R * scale);
            for (i, &y) in ys.iter().enumerate() {
                dots.push(SeatDot {
                    x: cx,
                    y,
                    r: SEAT_R * scale,
                    mine: mine_col == Some(col) && i == ys.len() / 2,
                });
            }
        }
        x += s.cols as f64 * scale + GALLEY;
    }

    let body_end = x - GALLEY;
    SeatMap {
        dots,
        nose,
        body_end,
        tip: body_end + tail,
        half: half_span + 0.55,
    }
}

/// Your seat on the aircraft's plan — the whole cabin drawn to scale, each
/// seat's footprint its share of the bill, yours in oxide.
#[component]
pub async fn seat_figure(cabin: Cabin, seat_fraction: f64, long_haul: bool) -> Result {
    let SeatMap {
        dots,
        nose,
        body_end,
        tip,
        half,
        ..
    } = seat_map(long_haul, cabin);
    let n_drawn = dots.len();
    let seats: Vec<(f64, f64, f64, bool)> = dots.iter().map(|d| (d.x, d.y, d.r, d.mine)).collect();

    // The airframe, hairline-quiet under the seats: a swept wing pair and
    // tailplane (stylized — real spans would swallow the rail), then the
    // fuselage on top so the roots stay hidden. Both edges sweep back —
    // the leading edge more — so the shapes read as airliner wings, not
    // deltas: the tip's leading edge starts aft of the root's trailing edge.
    let (wing_out, wing_sweep, wing_root, wing_tip) = if long_haul {
        (3.1, 5.4, 4.6, 1.5)
    } else {
        (2.3, 4.0, 3.4, 1.1)
    };
    let wx = tip * 0.4;
    let wing = |side: f64| {
        format!(
            "{},{} {},{} {},{} {},{}",
            f2(wx),
            f2(side * half * 0.8),
            f2(wx + wing_sweep),
            f2(side * (half + wing_out)),
            f2(wx + wing_sweep + wing_tip),
            f2(side * (half + wing_out)),
            f2(wx + wing_root),
            f2(side * half * 0.8),
        )
    };
    let tail_len = tip - body_end;
    let (stab_out, stab_root, stab_tip) = if long_haul {
        (1.9, 1.7, 0.55)
    } else {
        (1.5, 1.4, 0.5)
    };
    let sx = body_end + tail_len * 0.18;
    let stab = |side: f64| {
        format!(
            "{},{} {},{} {},{} {},{}",
            f2(sx),
            f2(side * half * 0.5),
            f2(sx + tail_len * 0.6),
            f2(side * (half * 0.5 + stab_out)),
            f2(sx + tail_len * 0.6 + stab_tip),
            f2(side * (half * 0.5 + stab_out)),
            f2(sx + stab_root),
            f2(side * half * 0.35),
        )
    };

    // Blunt round nose, straight body, tapering tail cone.
    let hull = format!(
        "M {} {} L {} {} C {} {} {} {} {} 0 C {} {} {} {} {} {} L {} {} C {} {} 0 {} 0 0 C 0 {} {} {} {} {} Z",
        f2(nose),
        f2(-half),
        f2(body_end),
        f2(-half),
        f2(body_end + tail_len * 0.45),
        f2(-half * 0.86),
        f2(body_end + tail_len * 0.82),
        f2(-half * 0.32),
        f2(tip),
        f2(body_end + tail_len * 0.82),
        f2(half * 0.32),
        f2(body_end + tail_len * 0.45),
        f2(half * 0.86),
        f2(body_end),
        f2(half),
        f2(nose),
        f2(half),
        f2(nose * 0.28),
        f2(half),
        f2(half * 0.55),
        f2(-half * 0.55),
        f2(nose * 0.28),
        f2(-half),
        f2(nose),
        f2(-half),
    );

    let extent = half + wing_out + 0.6;

    view! {
        <aside class="instrument" aria-label="Your seat">
            <p>
                "Your seat: "
                <strong>(format!("≈1/{}", format_whole(seat_fraction)))</strong>
                " of the aircraft "
                cite(id: "myclimate")
            </p>
            <svg
                viewBox=(format!("-0.5 {} {} {}", f2(-extent), f2(tip + 1.0), f2(extent * 2.0)))
                role="img"
                aria-label=(format!(
                    "Cabin plan: your {} seat highlighted among {} seats, each drawn to its share of the aircraft's bill",
                    cabin.as_str(),
                    format_whole(n_drawn as f64)
                ))
            >
                <g fill="var(--hairline)">
                    <polygon points=(wing(-1.0)) />
                    <polygon points=(wing(1.0)) />
                    <polygon points=(stab(-1.0)) />
                    <polygon points=(stab(1.0)) />
                </g>
                <path
                    d=(hull)
                    fill="var(--card)"
                    stroke="var(--muted)"
                    stroke-width="0.13"
                    stroke-linejoin="round"
                />
                for (x, y, r, mine) in seats {
                    if mine {
                        <circle cx=(f2(x)) cy=(f2(y)) r=(f2(r)) fill="var(--cost)" />
                    } else {
                        <circle cx=(f2(x)) cy=(f2(y)) r=(f2(r)) fill="var(--muted)" />
                    }
                }
            </svg>
            <div class="fig-note">
                "each seat drawn to its share of the bill"
            </div>
        </aside>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn great_circle_endpoints_and_continuity() {
        // JFK → LHR: no antimeridian, endpoints exact.
        let pts = great_circle_points(40.64, -73.78, 51.47, -0.46, 64);
        assert_eq!(pts.len(), 65);
        assert!((pts[0].0 - 40.64).abs() < 1e-9 && (pts[0].1 - -73.78).abs() < 1e-9);
        assert!((pts[64].0 - 51.47).abs() < 1e-6 && (pts[64].1 - -0.46).abs() < 1e-6);
        // A great circle north of both endpoints: the peak latitude exceeds LHR's.
        let peak = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        assert!(peak > 51.47);
    }

    #[test]
    fn antimeridian_route_stays_one_polyline() {
        // LAX → SYD crosses the antimeridian; unwrapped lons must never jump.
        let pts = great_circle_points(33.94, -118.41, -33.95, 151.18, 64);
        for w in pts.windows(2) {
            assert!(
                (w[1].1 - w[0].1).abs() < 180.0,
                "longitude jump {} → {}",
                w[0].1,
                w[1].1
            );
        }
        // The unwrapped run leaves the [-180, 180] band rather than snapping.
        let lon_min = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        assert!(lon_min < -180.0);
    }

    #[test]
    fn multi_leg_polyline_stays_continuous_across_joins() {
        // LAX → HNL → SYD: the second leg crosses the antimeridian, and its
        // sampling restarts from raw coordinates — the join must re-anchor
        // into the first leg's unwrapped run without a 360° jump.
        let stops = [(33.94, -118.41), (21.32, -157.92), (-33.95, 151.18)];
        let (pts, stop_at) = route_polyline(&stops, 64);
        assert_eq!(pts.len(), 2 * 64 + 1);
        assert_eq!(stop_at, vec![0, 64, 128]);
        for w in pts.windows(2) {
            assert!(
                (w[1].1 - w[0].1).abs() < 180.0,
                "longitude jump {} → {}",
                w[0].1,
                w[1].1
            );
        }
        // Each stop's polyline point is the stop itself, modulo unwrapping.
        for (&(lat, lon), &i) in stops.iter().zip(&stop_at) {
            assert!((pts[i].0 - lat).abs() < 1e-6);
            let dlon = (pts[i].1 - lon).rem_euclid(360.0);
            assert!(dlon.min(360.0 - dlon) < 1e-6, "lon {} vs {}", pts[i].1, lon);
        }
    }

    #[test]
    fn single_stop_route_plots_the_point() {
        let (pts, stop_at) = route_polyline(&[(40.64, -73.78)], 64);
        assert_eq!(pts, vec![(40.64, -73.78)]);
        assert_eq!(stop_at, vec![0]);
    }

    #[test]
    fn graticule_steps_stay_readable() {
        for span in [3.0, 12.0, 40.0, 90.0, 170.0, 300.0] {
            let step = graticule_step(span);
            assert!(
                span / step <= 6.0 || step == 60.0,
                "span {span} → step {step}"
            );
        }
        assert_eq!(graticule_step(12.0), 5.0);
        assert_eq!(graticule_step(100.0), 20.0);
    }

    #[test]
    fn tallies_format_like_the_receipt_would() {
        assert_eq!(format_tally(1.81), "1.8");
        assert_eq!(format_tally(1.0), "1");
        assert_eq!(format_tally(18.4), "18");
        assert_eq!(format_tally(0.18), "0.2");
        assert_eq!(format_tally(0.04), "0.04");
    }

    #[test]
    fn tally_fracs_fill_full_glyphs_then_the_remainder() {
        let f = tally_fracs(2.4);
        assert_eq!(f.len(), 3);
        assert!((f[0] - 1.0).abs() < 1e-9 && (f[1] - 1.0).abs() < 1e-9);
        assert!((f[2] - 0.4).abs() < 1e-9);
        // Whole counts draw no trailing empty glyph; zero draws one, empty.
        assert_eq!(tally_fracs(3.0), vec![1.0, 1.0, 1.0]);
        assert_eq!(tally_fracs(0.0), vec![0.0]);
        // The glyphs always sum back to the count — the tally never lies.
        let sum: f64 = tally_fracs(7.3).iter().sum();
        assert!((sum - 7.3).abs() < 1e-9);
    }

    #[test]
    fn driving_units_match_their_stated_bases() {
        // The captions and cites claim 2,790 mi at 35 mpg; 15 mi each way,
        // five days a week (≈21.7 commuting days a month) at 20 mpg; and a
        // 32-gallon tank — all at EPA's 8.887 kg CO₂/gal. Pin the
        // arithmetic so a factor edit can't drift from the copy. (That a
        // commute month and a Hummer tank land within 2% of each other is
        // a coincidence the captions don't lean on.)
        assert!((COAST_RUN_KG - 708.4).abs() < 0.5);
        assert!((COMMUTE_MONTH_KG - 288.8).abs() < 0.5);
        assert!((HUMMER_TANK_KG - 284.4).abs() < 0.1);
    }

    #[test]
    fn road_unit_showcases_the_largest_unit_past_three() {
        // One metric at a time: coast runs once the flight clears three of
        // them, else commute months once it clears three of those, else
        // Hummer tanks. Multipliers sit clear of the boundary so float
        // rounding can't flip a branch.
        assert_eq!(road_unit(COAST_RUN_KG * 3.1), RoadUnit::CoastRuns);
        assert_eq!(road_unit(COAST_RUN_KG * 2.9), RoadUnit::CommuteMonths);
        assert_eq!(road_unit(COMMUTE_MONTH_KG * 3.1), RoadUnit::CommuteMonths);
        assert_eq!(road_unit(COMMUTE_MONTH_KG * 2.9), RoadUnit::HummerTanks);
        assert_eq!(road_unit(0.0), RoadUnit::HummerTanks);
        // The months branch never needs a second year strip: below the
        // coast threshold the commute tally tops out around 7.4 months —
        // "the strip a year" stays singular. (Indexed through an array so
        // clippy's assertions_on_constants doesn't fold it away.)
        let cap = [COAST_RUN_KG * 3.0 / COMMUTE_MONTH_KG, 12.0];
        assert!(cap[0] < cap[1]);
    }

    #[test]
    fn route_window_caps_aspect_and_respects_poles() {
        // JFK → SCL is near-meridional: uncapped it towers ~13:1.
        let scl = great_circle_points(40.64, -73.78, -33.39, -70.79, 64);
        let w = route_window(&scl);
        assert!(w.vh / w.vw <= 1.16, "aspect {} still towers", w.vh / w.vw);
        // SFO → DXB arcs to 88°N; the padded window must stop at the pole.
        let dxb = great_circle_points(37.62, -122.38, 25.25, 55.36, 64);
        let w = route_window(&dxb);
        assert!(w.lat_hi <= 90.0 && w.lat_lo >= -90.0);
        assert!((w.y0 - -w.lat_hi).abs() < 1e-9);
    }

    #[test]
    fn svg_attribute_formatter_clips_noise() {
        assert_eq!(f2(0.8200000000000001), "0.82");
        assert_eq!(f2(-101.87309597671697), "-101.87");
    }

    #[test]
    fn seat_map_stays_honest_to_the_haul_models() {
        use super::super::emissions::seat_count;
        for long_haul in [false, true] {
            for cabin in [Cabin::Economy, Cabin::Business, Cabin::First] {
                let map = seat_map(long_haul, cabin);
                // Every seat's drawn area is its myclimate weight, so the
                // weights recovered from the radii must sum back to the
                // model's average seat count, or the plan misstates the
                // split.
                let w_econ = cabin_weight(long_haul, Cabin::Economy);
                let weighted: f64 = map
                    .dots
                    .iter()
                    .map(|d| (d.r / SEAT_R).powi(2) * w_econ)
                    .sum();
                let model = seat_count(long_haul);
                let err = ((weighted - model) / model).abs();
                assert!(
                    err < 0.01,
                    "haul {long_haul} cabin {}: drawn weight {weighted} vs model {model}",
                    cabin.as_str(),
                );
                // Exactly one seat is yours, drawn at your cabin's scale —
                // area proportional to its share of the bill.
                let mine: Vec<_> = map.dots.iter().filter(|d| d.mine).collect();
                assert_eq!(mine.len(), 1);
                let scale = (cabin_weight(long_haul, cabin)
                    / cabin_weight(long_haul, Cabin::Economy))
                .sqrt();
                assert!((mine[0].r - SEAT_R * scale).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn seat_dots_stay_inside_the_fuselage() {
        for long_haul in [false, true] {
            for cabin in [Cabin::Economy, Cabin::Business, Cabin::First] {
                let map = seat_map(long_haul, cabin);
                for d in &map.dots {
                    assert!(d.y.abs() + d.r < map.half, "seat pokes out the side");
                    assert!(
                        d.x - d.r > map.nose - 1e-9 && d.x + d.r < map.body_end + 1e-9,
                        "seat in the nose or tail cone"
                    );
                }
            }
        }
    }
}
