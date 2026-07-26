//! The charts of the planes post: the Cuts view (FlightScale — sliced
//! year-bars, dashed flight ticks, cut chips, the habit zoom panel and the
//! combined-swaps bar), the Compare view (ComparisonScale with makeup
//! chips), the ice callout beside them, and the travel-allowance
//! BudgetChart. Ported from `~/how-bad/src/components/{FlightScale,
//! ComparisonScale,IceCallout,IceGraphic,BudgetChart}.tsx` and the tab shell
//! in `App.tsx`.
//!
//! All arithmetic stays server-side: the cut chips and makeup chips write
//! plain signals, the affected chart regions are `#[shard]`s re-rendered on
//! the server, and the "erased" slice styling is pure CSS keyed off
//! reactive `data-pick-*` attributes (see `styles/planes-charts.css`).
//! Tooltips are CSS-only `.tip` spans on :hover/:focus-visible — no JS
//! positioning, unlike the original's tooltip layer.

use super::ice::{ICE_SHOW_FLOOR_M2, ice_figure};
use super::instruments::{driving_figure, fuel_figure, seat_figure};
use super::receipt;
use topcoat::{
    Result,
    runtime::shard,
    view::{component, view},
};

use super::{
    airports::Airport,
    comparison_scale::{
        ComparisonMode, comparison_rows, domain_color, list_makeup_chips, pick_makeup_row,
    },
    emissions::{
        Cabin, Coordinates, FlightImpact, JET_FUEL_KG_PER_LITRE, great_circle_km, route_impact,
    },
    format::{
        format_bar_value, format_count, format_ice, format_js_number, format_litres, format_tonnes,
        format_tonnes_smart, format_whole, format_years_span,
    },
    reference_data::{
        BUDGET_TARGETS, CutOption, FlightAnalogy, HABIT_BARS, SACRIFICE_BARS, SacrificeBar,
        cuttable_kg, pick_analogy, total_kg,
    },
    sources::cite,
};

fn all_bars() -> impl Iterator<Item = &'static SacrificeBar> {
    SACRIFICE_BARS.iter().chain(HABIT_BARS.iter())
}

fn find_bar(id: &str) -> &'static SacrificeBar {
    all_bars()
        .find(|b| b.id == id)
        .unwrap_or_else(|| panic!("unknown bar id: {id}"))
}

fn find_option(bar: &'static SacrificeBar, id: &str) -> &'static CutOption {
    bar.options
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("unknown option id: {id}"))
}

fn opt_label(bar_id: &str, option_id: &str) -> &'static str {
    find_option(find_bar(bar_id), option_id).label
}

/// What a cut erases: the summed weight of the slices it names.
fn option_kg(bar: &SacrificeBar, option_id: &str) -> f64 {
    let option = bar
        .options
        .iter()
        .find(|o| o.id == option_id)
        .expect("known option id");
    bar.slices
        .iter()
        .filter(|slice| slice.cut.is_some() && option.slice_ids.contains(&slice.id))
        .map(|slice| slice.kg)
        .sum()
}

/// The full monk year: every bar's deepest cut, summed, in tonnes.
fn monk_tonnes() -> f64 {
    all_bars().map(cuttable_kg).sum::<f64>() / 1000.0
}

/// A chip label without its price tag: "go vegan −1.0 t" → "go vegan".
fn strip_price(label: &str) -> &str {
    label.split(" −").next().unwrap_or(label)
}

/// Slice widths for a track, clamped so they never sum past 100%. Without
/// the clamp, a bar that outweighs its track (a habit year vs. a short
/// flight in the zoom panel) would flex-shrink every slice proportionally —
/// the track would still look full-width, but every slice's share and the
/// dashed tick's position would silently lie. Overflow past one track is
/// truncated instead, matching the Compare view's `.min(100.0)`.
fn clamped_widths(kgs: impl Iterator<Item = f64>, denom_kg: f64) -> Vec<f64> {
    let mut remaining = 100.0_f64;
    kgs.map(|kg| {
        let w = (kg / denom_kg * 100.0).min(remaining).max(0.0);
        remaining -= w;
        w
    })
    .collect()
}

/// A bar row's head: the year being dissected, its detail line and cites.
#[component]
async fn row_head(noun: &str, detail: &str, source_ids: Vec<&'static str>) -> Result {
    view! {
        <span class="row-head">
            (noun)
            <span class="row-detail">
                (detail)
                " "
                for id in source_ids {
                    cite(id: id)
                }
            </span>
        </span>
    }
}

/// One sacrifice bar's track: its slices (each with a CSS tooltip and a
/// `seg-{bar}-{slice}` class the erased styling keys on) plus the labeled
/// dashed flight tick priced in the bar's own currency.
#[component]
async fn bar_track(
    bar_id: &str,
    flight_kg: f64,
    denom_kg: f64,
    tonnes_tips: bool,
    seed_offset: i64,
    tick_pct: f64,
    trip_noun: String,
) -> Result {
    let bar = find_bar(bar_id);
    let has_tick = flight_kg > 0.0;
    let (tick_text, tick_tip) = if has_tick {
        let seed = flight_kg.round() as i64 + seed_offset;
        let (analogy, count) = pick_analogy(bar.analogies, flight_kg, seed);
        (
            analogy.tick.replace("{n}", &format_count(count)),
            format!(
                "This {trip_noun} ≈ {} {} ({})",
                format_whole(count),
                analogy.unit_label,
                analogy.basis
            ),
        )
    } else {
        (String::new(), String::new())
    };
    let flip = tick_pct > 55.0;

    struct Seg {
        class: String,
        style: String,
        tip: String,
    }
    let widths = clamped_widths(bar.slices.iter().map(|s| s.kg), denom_kg);
    let segs: Vec<Seg> = bar
        .slices
        .iter()
        .zip(widths)
        .map(|(slice, width)| Seg {
            class: format!("bar-seg seg-{}-{}", bar.id, slice.id),
            style: format!("width:{width}%;background:{}", slice.color),
            tip: format!(
                "{} — {}",
                if tonnes_tips {
                    format_tonnes_smart(slice.kg / 1000.0)
                } else {
                    format_bar_value(slice.kg)
                },
                slice.label
            ),
        })
        .collect();

    view! {
        <div class="bar-h-track">
            for seg in segs {
                <div class=(seg.class) tabindex="0" style=(seg.style)>
                    <span class="tip">(seg.tip)</span>
                </div>
            }
            if has_tick {
                <span class="flight-tick" style=(format!("left:{tick_pct}%"))>
                    <span class=(if flip { "tick-label flip" } else { "tick-label" }) tabindex="0">
                        (tick_text)
                        <span class="tip">(tick_tip)</span>
                    </span>
                </span>
            }
        </div>
    }
}

/// The "…but what about?" track: every habit bar's slices end to end, with
/// the flight quoted in one of the habits' own currencies.
#[component]
async fn habits_track(
    flight_kg: f64,
    denom_kg: f64,
    seed_offset: i64,
    tick_pct: f64,
    trip_noun: String,
) -> Result {
    struct Seg {
        class: String,
        style: String,
        tip: String,
    }
    let slices: Vec<_> = HABIT_BARS
        .iter()
        .flat_map(|bar| bar.slices.iter().map(move |slice| (bar, slice)))
        .collect();
    let widths = clamped_widths(slices.iter().map(|(_, s)| s.kg), denom_kg);
    let segs: Vec<Seg> = slices
        .iter()
        .zip(widths)
        .map(|((bar, slice), width)| Seg {
            class: format!("bar-seg seg-{}-{}", bar.id, slice.id),
            style: format!("width:{width}%;background:{}", slice.color),
            tip: format!("{} — {}", format_bar_value(slice.kg), slice.label),
        })
        .collect();

    let pool: Vec<FlightAnalogy> = HABIT_BARS
        .iter()
        .flat_map(|bar| bar.analogies.iter().copied())
        .collect();
    let has_tick = flight_kg > 0.0;
    let (tick_text, tick_tip) = if has_tick {
        let seed = flight_kg.round() as i64 + seed_offset;
        let (analogy, count) = pick_analogy(&pool, flight_kg, seed);
        (
            analogy.tick.replace("{n}", &format_count(count)),
            format!(
                "This {trip_noun} ≈ {} {} ({})",
                format_whole(count),
                analogy.unit_label,
                analogy.basis
            ),
        )
    } else {
        (String::new(), String::new())
    };
    let flip = tick_pct > 55.0;

    view! {
        <div class="bar-h-track">
            for seg in segs {
                <div class=(seg.class) tabindex="0" style=(seg.style)>
                    <span class="tip">(seg.tip)</span>
                </div>
            }
            if has_tick {
                <span class="flight-tick" style=(format!("left:{tick_pct}%"))>
                    <span class=(if flip { "tick-label flip" } else { "tick-label" }) tabindex="0">
                        (tick_text)
                        <span class="tip">(tick_tip)</span>
                    </span>
                </span>
            }
        </div>
    }
}

/// The "Your swaps, combined" bar. The picks arrive as one signal per cut
/// ladder (0 = untouched, 1 = the mild cut, 2 = the deep one; the six
/// single-cut habits packed into decimal digits of `habits`) so the shard
/// can re-price everything server-side.
#[shard]
async fn combined_swaps(
    flight_kg: f64,
    heat: f64,
    cool: f64,
    diet: f64,
    waste: f64,
    fashion: f64,
    coffee: f64,
    habits: f64,
) -> Result {
    let monk_t = monk_tonnes();
    let flight_tonnes = flight_kg / 1000.0;
    let scale_max = flight_tonnes.max(monk_t);
    let flight_pct = flight_tonnes / scale_max * 100.0;
    let h = habits.round() as i64;
    let ladder = |bar_id: &str, group: Option<&str>| -> i64 {
        match (bar_id, group) {
            ("climate", Some("heat")) => heat.round() as i64,
            ("climate", Some("cool")) => cool.round() as i64,
            ("eating", Some("diet")) => diet.round() as i64,
            ("eating", Some("waste")) => waste.round() as i64,
            ("fashion", None) => fashion.round() as i64,
            ("coffee", None) => coffee.round() as i64,
            ("phone", None) => h % 10,
            ("soda", None) => (h / 10) % 10,
            ("bottled-water", None) => (h / 100) % 10,
            ("streaming", None) => (h / 1000) % 10,
            ("chatgpt", None) => (h / 10_000) % 10,
            ("straws", None) => (h / 100_000) % 10,
            _ => 0,
        }
    };

    struct Seg {
        style: String,
        tip: String,
    }
    let mut segs = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut picked_kg = 0.0;
    for bar in all_bars() {
        for option in bar.options.iter() {
            // Options run mild→deep within a ladder; a pick is the option's
            // 1-based position among its group.
            let pos = bar
                .options
                .iter()
                .filter(|o| o.group == option.group)
                .position(|o| o.id == option.id)
                .map(|p| p as i64 + 1)
                .unwrap_or(0);
            if pos == 0 || ladder(bar.id, option.group) != pos {
                continue;
            }
            let kg = option_kg(bar, option.id);
            let color = bar
                .slices
                .iter()
                .find(|s| s.id == option.slice_ids[0])
                .map(|s| s.color)
                .unwrap_or("var(--ink)");
            picked_kg += kg;
            labels.push(strip_price(option.label).to_string());
            segs.push(Seg {
                style: format!(
                    "width:{}%;background:{}",
                    kg / 1000.0 / scale_max * 100.0,
                    color
                ),
                tip: format!(
                    "{} — {}: −{}",
                    bar.noun,
                    strip_price(option.label),
                    format_bar_value(kg)
                ),
            });
        }
    }
    let picked_tonnes = picked_kg / 1000.0;
    let cuts_text = if labels.is_empty() {
        "empty until you tap a cut — see how much it takes to cross the dashed line".to_string()
    } else {
        labels.join(" + ")
    };
    let equals = if picked_tonnes > 0.0 {
        format_tonnes_smart(picked_tonnes)
    } else {
        "0.0 t".to_string()
    };

    view! {
        <div class="bar-cell">
            <div class="bar-h-track">
                for seg in segs {
                    <div class="bar-seg" tabindex="0" style=(seg.style)>
                        <span class="tip">(seg.tip)</span>
                    </div>
                }
                if flight_tonnes > 0.0 {
                    <span
                        class="flight-tick"
                        style=(format!("left:{flight_pct}%"))
                        aria-hidden="true"
                    ></span>
                }
            </div>
            <p class="row-cuts">(cuts_text)</p>
        </div>
        <span class="row-equals">
            <strong>(equals)</strong>
            <span class="row-sub">(format!("of {} possible", format_tonnes(monk_t)))</span>
        </span>
    }
}

/// The Compare view's domain rows. Each makeup chip writes its domain's
/// signal (the chip id, or "" for none); the rows recompute server-side.
#[shard]
async fn compare_domain_rows(
    scale_kg: f64,
    transport: String,
    food: String,
    home: String,
    habits: String,
) -> Result {
    let scale = if scale_kg > 0.0 { scale_kg } else { 1.0 };
    let chips = list_makeup_chips();
    let rows: Vec<_> = comparison_rows(scale, ComparisonMode::Absolute)
        .into_iter()
        .map(|row| {
            let chip_id = match row.domain.as_str() {
                "transport" => transport.as_str(),
                "food" => food.as_str(),
                "home" => home.as_str(),
                "habits" => habits.as_str(),
                _ => "",
            };
            if chip_id.is_empty() {
                return row;
            }
            match chips
                .iter()
                .find(|c| c.id == chip_id && c.domain == row.domain)
            {
                Some(chip) => pick_makeup_row(scale, chip, ComparisonMode::Absolute, None),
                None => row,
            }
        })
        .collect();

    view! {
        for row in rows {
            <div class="effort-row">
                <span class="row-head">
                    (row.label)
                    <span class="row-detail">
                        (row.detail)
                        " "
                        for id in row.source_ids.iter() {
                            cite(id: id.as_str())
                        }
                    </span>
                </span>
                <div class="bar-cell">
                    <div class="bar-h-track">
                        <div
                            class="bar-seg"
                            tabindex="0"
                            style=(format!(
                                "width:{}%;background:{}",
                                (row.bar_fill_kg / scale * 100.0).min(100.0),
                                domain_color(&row.domain).unwrap_or("var(--ink)")
                            ))
                        >
                            <span class="tip">(format!(
                                "{} — {} {}",
                                format_bar_value(row.bar_fill_kg),
                                format_js_number(row.count),
                                row.unit_label
                            ))</span>
                        </div>
                    </div>
                </div>
                <span class="row-equals">
                    <strong>(format_bar_value(row.bar_fill_kg))</strong>
                </span>
            </div>
        }
    }
}

/// The sticky sidebar beside the charts: the melted-ice line and figure.
#[component]
async fn ice_callout(ice_m2: f64) -> Result {
    view! {
        <aside class="ice-callout instrument" aria-label="Arctic sea ice">
            <p>
                "Arctic sea ice melted: "
                <strong>(format_ice(ice_m2))</strong>
                " "
                cite(id: "seaice")
            </p>
            ice_figure(ice_m2: ice_m2, pattern_id: "callout-ice-dots-pat")
        </aside>
    }
}

/// The Cuts view's `<details>` tables: every slice with what erases it, and
/// every currency the dashed line could have quoted.
#[component]
async fn cuts_data_tables(flight_tonnes: f64, trip_noun: String) -> Result {
    let flight_kg = flight_tonnes * 1000.0;
    let monk_t = monk_tonnes();

    struct SliceRow {
        year: &'static str,
        slice: String,
        erased_by: &'static str,
        tonnes: String,
    }
    let mut slice_rows = Vec::new();
    for bar in all_bars() {
        for (i, slice) in bar.slices.iter().enumerate() {
            slice_rows.push(SliceRow {
                year: if i == 0 { bar.noun } else { "" },
                slice: slice.id.replace('-', " "),
                erased_by: slice.cut.unwrap_or("nothing — this floor stays"),
                tonnes: format!("{:.3}", slice.kg / 1000.0),
            });
        }
    }

    struct AnalogyRow {
        year: &'static str,
        quote: String,
        basis: &'static str,
        source_ids: &'static [&'static str],
    }
    let mut analogy_rows = Vec::new();
    for bar in all_bars() {
        for (i, analogy) in bar.analogies.iter().enumerate() {
            analogy_rows.push(AnalogyRow {
                year: if i == 0 { bar.noun } else { "" },
                quote: format!(
                    "{} {}",
                    format_whole(flight_kg / analogy.kg_per_unit),
                    analogy.unit_label
                ),
                basis: analogy.basis,
                source_ids: analogy.source_ids,
            });
        }
    }

    view! {
        <details class="data-table">
            <summary>"Data table"</summary>
            <table>
                <thead>
                    <tr>
                        <th>"The year"</th>
                        <th>"The slice"</th>
                        <th>"Erased by"</th>
                        <th>"tCO₂e"</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td>"This flight (one seat)"</td>
                        <td>"—"</td>
                        <td>"staying grounded"</td>
                        <td>(format!("{flight_tonnes:.2}"))</td>
                    </tr>
                    for r in slice_rows {
                        <tr>
                            <td>(r.year)</td>
                            <td>(r.slice)</td>
                            <td>(r.erased_by)</td>
                            <td>(r.tonnes)</td>
                        </tr>
                    }
                    <tr>
                        <td>"The full monk year"</td>
                        <td>"every cuttable slice"</td>
                        <td>"all of it, at once"</td>
                        <td>(format!("{monk_t:.2}"))</td>
                    </tr>
                </tbody>
            </table>
            <p class="toggle-note table-note">
                "…and every currency the dashed line could have quoted, exact:"
            </p>
            <table>
                <thead>
                    <tr>
                        <th>"The year"</th>
                        <th>(format!("This {trip_noun} ≈"))</th>
                        <th>"Basis"</th>
                    </tr>
                </thead>
                <tbody>
                    for r in analogy_rows {
                        <tr>
                            <td>(r.year)</td>
                            <td>(r.quote)</td>
                            <td>
                                (r.basis)
                                " "
                                for id in r.source_ids.iter().copied() {
                                    cite(id: id)
                                }
                            </td>
                        </tr>
                    }
                </tbody>
            </table>
        </details>
    }
}

/// "The flight vs. everything else": the Cuts / Compare / Receipt / Allowance
/// tab shell, the chart cards (one visible at a time via a signal;
/// `initial_view` is `"cuts"`, `"compare"`, `"receipt"`, or `"allowance"`),
/// and the ice callout beside the chart views.
#[component]
pub async fn charts_section(
    impact: FlightImpact,
    round_trip: bool,
    from: Airport,
    vias: Vec<Airport>,
    to: Airport,
    cabin: Cabin,
    share_path: String,
    initial_view: String,
) -> Result {
    let to_city = to.city.clone();
    let flight_tonnes = impact.tonnes_co2e;
    let stops: Vec<Coordinates> = std::iter::once(&from)
        .chain(vias.iter())
        .chain(std::iter::once(&to))
        .map(Airport::coordinates)
        .collect();
    let cabin_years = |cabin: Cabin| route_impact(&stops, cabin, round_trip).travel_budget_years;
    let flight_kg = flight_tonnes * 1000.0;
    let monk_t = monk_tonnes();
    let scale_max = flight_tonnes.max(monk_t);
    let scale_max_kg = scale_max * 1000.0;
    let flight_pct = flight_tonnes / scale_max * 100.0;
    let trip_noun = if round_trip { "round trip" } else { "flight" };
    let habits_kg: f64 = HABIT_BARS.iter().map(total_kg).sum();
    let habit_max = HABIT_BARS.iter().map(total_kg).fold(0.0_f64, f64::max);
    // The zoom panel's scale: the track is exactly one flight wide.
    let zoom_kg = if flight_kg > 0.0 {
        flight_kg
    } else {
        habit_max
    };
    let show_ice = impact.ice_m2 >= ICE_SHOW_FLOOR_M2;
    // The margin instruments need a real route; a staycation has no rail.
    let show_rail = impact.distance_km > 0.0 || show_ice;
    let fuel_litres = impact.fuel_kg / JET_FUEL_KG_PER_LITRE;
    let seat_fraction = if impact.seat_share_of_aircraft > 0.0 {
        (1.0 / impact.seat_share_of_aircraft).round()
    } else {
        1.0
    };
    // The seat map depicts the nearer haul model; 2000 km is the midpoint of
    // the 1500–2500 km blend zone the fuel model interpolates across. With
    // layovers each leg flies its own aircraft — the longest leg (the bulk
    // of the bill) picks the one drawn.
    let longest_leg_km = stops
        .windows(2)
        .map(|w| great_circle_km(w[0].lat, w[0].lon, w[1].lat, w[1].lon))
        .fold(0.0_f64, f64::max);
    let long_haul = longest_leg_km >= 2000.0;
    let sac_len = SACRIFICE_BARS.len() as i64;
    let start_view = initial_view.clone();

    let b_climate = find_bar("climate");
    let b_eating = find_bar("eating");
    let b_fashion = find_bar("fashion");
    let b_coffee = find_bar("coffee");
    let b_phone = find_bar("phone");
    let b_soda = find_bar("soda");
    let b_bottle = find_bar("bottled-water");
    let b_stream = find_bar("streaming");
    let b_gpt = find_bar("chatgpt");
    let b_straw = find_bar("straws");

    let labeled_cell = if flight_tonnes > 0.0 {
        "bar-cell labeled"
    } else {
        "bar-cell"
    };

    view! {
        <section class="section">
            signal view = start_view;
            signal zoom = false;
            signal fkg = flight_kg;
            signal p_heat = 0.0;
            signal p_cool = 0.0;
            signal p_diet = 0.0;
            signal p_waste = 0.0;
            signal p_fashion = 0.0;
            signal p_coffee = 0.0;
            signal p_phone = 0.0;
            signal p_soda = 0.0;
            signal p_bottle = 0.0;
            signal p_stream = 0.0;
            signal p_gpt = 0.0;
            signal p_straw = 0.0;
            signal m_transport = String::new();
            signal m_food = String::new();
            signal m_home = String::new();
            signal m_habits = String::new();

            <h2>"The flight vs. everything else"</h2>
            <div class="chart-tabs" role="tablist" aria-label="Chart view">
                <button
                    type="button"
                    role="tab"
                    id="chart-tab-cuts"
                    aria-controls="chart-panel"
                    :aria-selected=$(if view.get() == "cuts" { "true" } else { "false" })
                    :class=$(if view.get() == "cuts" { "chart-tab is-active" } else { "chart-tab" })
                    @click=$(|_e| view.set("cuts".to_owned()))
                >"Cuts"</button>
                <button
                    type="button"
                    role="tab"
                    id="chart-tab-compare"
                    aria-controls="chart-panel"
                    :aria-selected=$(if view.get() == "compare" { "true" } else { "false" })
                    :class=$(if view.get() == "compare" { "chart-tab is-active" } else { "chart-tab" })
                    @click=$(|_e| view.set("compare".to_owned()))
                >"Compare"</button>
                <button
                    type="button"
                    role="tab"
                    id="chart-tab-receipt"
                    aria-controls="chart-panel"
                    :aria-selected=$(if view.get() == "receipt" { "true" } else { "false" })
                    :class=$(if view.get() == "receipt" { "chart-tab is-active" } else { "chart-tab" })
                    @click=$(|_e| view.set("receipt".to_owned()))
                >"Receipt"</button>
                <button
                    type="button"
                    role="tab"
                    id="chart-tab-allowance"
                    aria-controls="chart-panel"
                    :aria-selected=$(if view.get() == "allowance" { "true" } else { "false" })
                    :class=$(if view.get() == "allowance" { "chart-tab is-active" } else { "chart-tab" })
                    @click=$(|_e| view.set("allowance".to_owned()))
                >"Allowance"</button>
            </div>
            <div
                :class=$(
                    if view.get() == "receipt" {
                        "scale-layout"
                    } else if view.get() == "allowance" {
                        "scale-layout"
                    } else if show_rail {
                        "scale-layout scale-layout--ice"
                    } else {
                        "scale-layout"
                    }
                )
                id="chart-panel"
                role="tabpanel"
                :aria-labelledby=$(
                    if view.get() == "receipt" {
                        "chart-tab-receipt"
                    } else if view.get() == "allowance" {
                        "chart-tab-allowance"
                    } else if view.get() == "compare" {
                        "chart-tab-compare"
                    } else {
                        "chart-tab-cuts"
                    }
                )
            >
                <div class="scale-main">
                    <div :hidden=$(view.get() != "cuts")>
                        <div class="chart-card">
                            <p class="toggle-note">
                                "the dashed line quotes the flight in each row’s own units, \
                                 rounded to friendly numbers — hover a label for the exact count"
                            </p>

                            <div
                                class="effort-rows"
                                :data-pick-heat=$(p_heat.get())
                                :data-pick-cool=$(p_cool.get())
                                :data-pick-diet=$(p_diet.get())
                                :data-pick-waste=$(p_waste.get())
                                :data-pick-fashion=$(p_fashion.get())
                                :data-pick-coffee=$(p_coffee.get())
                                :data-pick-phone=$(p_phone.get())
                                :data-pick-soda=$(p_soda.get())
                                :data-pick-bottle=$(p_bottle.get())
                                :data-pick-stream=$(p_stream.get())
                                :data-pick-gpt=$(p_gpt.get())
                                :data-pick-straw=$(p_straw.get())
                            >
                                <div class="effort-row">
                                    <span class="row-head">
                                        <strong>"This flight"</strong>
                                        <span class="row-detail">
                                            (format!("one {trip_noun} to {to_city}, one seat\u{a0}"))
                                            cite(id: "myclimate")
                                            cite(id: "lee2021")
                                        </span>
                                    </span>
                                    <div class="bar-cell">
                                        <div class="bar-h-track">
                                            <div
                                                class="bar-seg"
                                                tabindex="0"
                                                style=(format!("width:{flight_pct}%;background:var(--cost)"))
                                            >
                                                <span class="tip">(format!(
                                                    "One {trip_noun} to {to_city}: {} CO₂e",
                                                    format_tonnes(flight_tonnes)
                                                ))</span>
                                            </div>
                                        </div>
                                    </div>
                                    <span class="row-equals">
                                        <strong>(format_tonnes(flight_tonnes))</strong>
                                    </span>
                                </div>
                                <hr class="flight-row-divider">

                                <div class="effort-row">
                                    row_head(
                                        noun: b_climate.noun,
                                        detail: b_climate.detail,
                                        source_ids: b_climate.source_ids.to_vec(),
                                    )
                                    <div class=(labeled_cell)>
                                        bar_track(
                                            bar_id: "climate",
                                            flight_kg: flight_kg,
                                            denom_kg: scale_max_kg,
                                            tonnes_tips: true,
                                            seed_offset: 1,
                                            tick_pct: flight_pct,
                                            trip_noun: trip_noun.to_string(),
                                        )
                                        <div
                                            class="cut-chips"
                                            role="group"
                                            aria-label=(format!("Cuts for {}", b_climate.noun))
                                        >
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_heat.get() == 1.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_heat.get() == 1.0 { p_heat.set(0.0) } else { p_heat.set(1.0) })
                                            >(opt_label("climate", "thermostat"))</button>
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_heat.get() == 2.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_heat.get() == 2.0 { p_heat.set(0.0) } else { p_heat.set(2.0) })
                                            >(opt_label("climate", "heat-pump"))</button>
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_cool.get() == 1.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_cool.get() == 1.0 { p_cool.set(0.0) } else { p_cool.set(1.0) })
                                            >(opt_label("climate", "sweat"))</button>
                                        </div>
                                    </div>
                                    <span class="row-equals">
                                        <strong>(format_tonnes_smart(total_kg(b_climate) / 1000.0))</strong>
                                    </span>
                                </div>

                                <div class="effort-row">
                                    row_head(
                                        noun: b_eating.noun,
                                        detail: b_eating.detail,
                                        source_ids: b_eating.source_ids.to_vec(),
                                    )
                                    <div class=(labeled_cell)>
                                        bar_track(
                                            bar_id: "eating",
                                            flight_kg: flight_kg,
                                            denom_kg: scale_max_kg,
                                            tonnes_tips: true,
                                            seed_offset: 2,
                                            tick_pct: flight_pct,
                                            trip_noun: trip_noun.to_string(),
                                        )
                                        <div
                                            class="cut-chips"
                                            role="group"
                                            aria-label=(format!("Cuts for {}", b_eating.noun))
                                        >
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_diet.get() == 1.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_diet.get() == 1.0 { p_diet.set(0.0) } else { p_diet.set(1.0) })
                                            >(opt_label("eating", "vegetarian"))</button>
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_diet.get() == 2.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_diet.get() == 2.0 { p_diet.set(0.0) } else { p_diet.set(2.0) })
                                            >(opt_label("eating", "vegan"))</button>
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_waste.get() == 1.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_waste.get() == 1.0 { p_waste.set(0.0) } else { p_waste.set(1.0) })
                                            >(opt_label("eating", "compost"))</button>
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_waste.get() == 2.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_waste.get() == 2.0 { p_waste.set(0.0) } else { p_waste.set(2.0) })
                                            >(opt_label("eating", "zero-waste"))</button>
                                        </div>
                                    </div>
                                    <span class="row-equals">
                                        <strong>(format_tonnes_smart(total_kg(b_eating) / 1000.0))</strong>
                                    </span>
                                </div>

                                <div class="effort-row">
                                    row_head(
                                        noun: b_fashion.noun,
                                        detail: b_fashion.detail,
                                        source_ids: b_fashion.source_ids.to_vec(),
                                    )
                                    <div class=(labeled_cell)>
                                        bar_track(
                                            bar_id: "fashion",
                                            flight_kg: flight_kg,
                                            denom_kg: scale_max_kg,
                                            tonnes_tips: true,
                                            seed_offset: 3,
                                            tick_pct: flight_pct,
                                            trip_noun: trip_noun.to_string(),
                                        )
                                        <div
                                            class="cut-chips"
                                            role="group"
                                            aria-label=(format!("Cuts for {}", b_fashion.noun))
                                        >
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_fashion.get() == 1.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_fashion.get() == 1.0 { p_fashion.set(0.0) } else { p_fashion.set(1.0) })
                                            >(opt_label("fashion", "slow-down"))</button>
                                            <button
                                                type="button"
                                                class="cut-chip"
                                                :aria-pressed=$(if p_fashion.get() == 2.0 { "true" } else { "false" })
                                                @click=$(|_e| if p_fashion.get() == 2.0 { p_fashion.set(0.0) } else { p_fashion.set(2.0) })
                                            >(opt_label("fashion", "thrift"))</button>
                                        </div>
                                    </div>
                                    <span class="row-equals">
                                        <strong>(format_tonnes_smart(total_kg(b_fashion) / 1000.0))</strong>
                                    </span>
                                </div>

                                <div class="effort-row">
                                    <span class="row-head">
                                        "…but what about?"
                                        <span class="row-detail">
                                            "lattes, new phones, soda, bottled water, streaming, ChatGPT, straws "
                                            cite(id: "items")
                                            cite(id: "soda")
                                            cite(id: "phone")
                                            cite(id: "streaming")
                                            cite(id: "ai-openai")
                                        </span>
                                    </span>
                                    <div class=(labeled_cell)>
                                        habits_track(
                                            flight_kg: flight_kg,
                                            denom_kg: scale_max_kg,
                                            seed_offset: sac_len,
                                            tick_pct: flight_pct,
                                            trip_noun: trip_noun.to_string(),
                                        )
                                        <p class="row-cuts">
                                            "all seven together, a few pixels at this scale — "
                                            <button
                                                type="button"
                                                class="link-btn"
                                                :hidden=$(zoom.get())
                                                :aria-expanded=$(if zoom.get() { "true" } else { "false" })
                                                @click=$(|_e| zoom.set(true))
                                            >"hold them up against the flight"</button>
                                            <span :hidden=$(!zoom.get())>"held up against the flight below ↓"</span>
                                        </p>
                                    </div>
                                    <span class="row-equals">
                                        <strong>(format_tonnes_smart(habits_kg / 1000.0))</strong>
                                    </span>
                                </div>

                                <div class="zoom-panel" :hidden=$(!zoom.get())>
                                    <div class="zoom-head">
                                        <p class="zoom-note">(format!(
                                            "the track is now exactly one {trip_noun} wide — the dashed line is this flight"
                                        ))</p>
                                        <button
                                            type="button"
                                            class="link-btn zoom-close"
                                            @click=$(|_e| zoom.set(false))
                                        >"put them back ↑"</button>
                                    </div>

                                    <div class="effort-row">
                                        row_head(
                                            noun: b_coffee.noun,
                                            detail: b_coffee.detail,
                                            source_ids: b_coffee.source_ids.to_vec(),
                                        )
                                        <div class=(labeled_cell)>
                                            bar_track(
                                                bar_id: "coffee",
                                                flight_kg: flight_kg,
                                                denom_kg: zoom_kg,
                                                tonnes_tips: false,
                                                seed_offset: sac_len + 1,
                                                tick_pct: 100.0,
                                                trip_noun: trip_noun.to_string(),
                                            )
                                            <div
                                                class="cut-chips"
                                                role="group"
                                                aria-label=(format!("Cuts for {}", b_coffee.noun))
                                            >
                                                <button
                                                    type="button"
                                                    class="cut-chip"
                                                    :aria-pressed=$(if p_coffee.get() == 1.0 { "true" } else { "false" })
                                                    @click=$(|_e| if p_coffee.get() == 1.0 { p_coffee.set(0.0) } else { p_coffee.set(1.0) })
                                                >(opt_label("coffee", "black"))</button>
                                                <button
                                                    type="button"
                                                    class="cut-chip"
                                                    :aria-pressed=$(if p_coffee.get() == 2.0 { "true" } else { "false" })
                                                    @click=$(|_e| if p_coffee.get() == 2.0 { p_coffee.set(0.0) } else { p_coffee.set(2.0) })
                                                >(opt_label("coffee", "quit"))</button>
                                            </div>
                                        </div>
                                        <span class="row-equals">
                                            <strong>(format_bar_value(total_kg(b_coffee)))</strong>
                                        </span>
                                    </div>

                                    <div class="effort-row">
                                        row_head(
                                            noun: b_phone.noun,
                                            detail: b_phone.detail,
                                            source_ids: b_phone.source_ids.to_vec(),
                                        )
                                        <div class=(labeled_cell)>
                                            bar_track(
                                                bar_id: "phone",
                                                flight_kg: flight_kg,
                                                denom_kg: zoom_kg,
                                                tonnes_tips: false,
                                                seed_offset: sac_len + 2,
                                                tick_pct: 100.0,
                                                trip_noun: trip_noun.to_string(),
                                            )
                                            <div
                                                class="cut-chips"
                                                role="group"
                                                aria-label=(format!("Cuts for {}", b_phone.noun))
                                            >
                                                <button
                                                    type="button"
                                                    class="cut-chip"
                                                    :aria-pressed=$(if p_phone.get() == 1.0 { "true" } else { "false" })
                                                    @click=$(|_e| if p_phone.get() == 1.0 { p_phone.set(0.0) } else { p_phone.set(1.0) })
                                                >(opt_label("phone", "keep"))</button>
                                            </div>
                                        </div>
                                        <span class="row-equals">
                                            <strong>(format_bar_value(total_kg(b_phone)))</strong>
                                        </span>
                                    </div>

                                    <div class="effort-row">
                                        row_head(
                                            noun: b_soda.noun,
                                            detail: b_soda.detail,
                                            source_ids: b_soda.source_ids.to_vec(),
                                        )
                                        <div class=(labeled_cell)>
                                            bar_track(
                                                bar_id: "soda",
                                                flight_kg: flight_kg,
                                                denom_kg: zoom_kg,
                                                tonnes_tips: false,
                                                seed_offset: sac_len + 3,
                                                tick_pct: 100.0,
                                                trip_noun: trip_noun.to_string(),
                                            )
                                            <div
                                                class="cut-chips"
                                                role="group"
                                                aria-label=(format!("Cuts for {}", b_soda.noun))
                                            >
                                                <button
                                                    type="button"
                                                    class="cut-chip"
                                                    :aria-pressed=$(if p_soda.get() == 1.0 { "true" } else { "false" })
                                                    @click=$(|_e| if p_soda.get() == 1.0 { p_soda.set(0.0) } else { p_soda.set(1.0) })
                                                >(opt_label("soda", "quit"))</button>
                                            </div>
                                        </div>
                                        <span class="row-equals">
                                            <strong>(format_bar_value(total_kg(b_soda)))</strong>
                                        </span>
                                    </div>

                                    <div class="effort-row">
                                        row_head(
                                            noun: b_bottle.noun,
                                            detail: b_bottle.detail,
                                            source_ids: b_bottle.source_ids.to_vec(),
                                        )
                                        <div class=(labeled_cell)>
                                            bar_track(
                                                bar_id: "bottled-water",
                                                flight_kg: flight_kg,
                                                denom_kg: zoom_kg,
                                                tonnes_tips: false,
                                                seed_offset: sac_len + 4,
                                                tick_pct: 100.0,
                                                trip_noun: trip_noun.to_string(),
                                            )
                                            <div
                                                class="cut-chips"
                                                role="group"
                                                aria-label=(format!("Cuts for {}", b_bottle.noun))
                                            >
                                                <button
                                                    type="button"
                                                    class="cut-chip"
                                                    :aria-pressed=$(if p_bottle.get() == 1.0 { "true" } else { "false" })
                                                    @click=$(|_e| if p_bottle.get() == 1.0 { p_bottle.set(0.0) } else { p_bottle.set(1.0) })
                                                >(opt_label("bottled-water", "quit"))</button>
                                            </div>
                                        </div>
                                        <span class="row-equals">
                                            <strong>(format_bar_value(total_kg(b_bottle)))</strong>
                                        </span>
                                    </div>

                                    <div class="effort-row">
                                        row_head(
                                            noun: b_stream.noun,
                                            detail: b_stream.detail,
                                            source_ids: b_stream.source_ids.to_vec(),
                                        )
                                        <div class=(labeled_cell)>
                                            bar_track(
                                                bar_id: "streaming",
                                                flight_kg: flight_kg,
                                                denom_kg: zoom_kg,
                                                tonnes_tips: false,
                                                seed_offset: sac_len + 5,
                                                tick_pct: 100.0,
                                                trip_noun: trip_noun.to_string(),
                                            )
                                            <div
                                                class="cut-chips"
                                                role="group"
                                                aria-label=(format!("Cuts for {}", b_stream.noun))
                                            >
                                                <button
                                                    type="button"
                                                    class="cut-chip"
                                                    :aria-pressed=$(if p_stream.get() == 1.0 { "true" } else { "false" })
                                                    @click=$(|_e| if p_stream.get() == 1.0 { p_stream.set(0.0) } else { p_stream.set(1.0) })
                                                >(opt_label("streaming", "quit"))</button>
                                            </div>
                                        </div>
                                        <span class="row-equals">
                                            <strong>(format_bar_value(total_kg(b_stream)))</strong>
                                        </span>
                                    </div>

                                    <div class="effort-row">
                                        row_head(
                                            noun: b_gpt.noun,
                                            detail: b_gpt.detail,
                                            source_ids: b_gpt.source_ids.to_vec(),
                                        )
                                        <div class=(labeled_cell)>
                                            bar_track(
                                                bar_id: "chatgpt",
                                                flight_kg: flight_kg,
                                                denom_kg: zoom_kg,
                                                tonnes_tips: false,
                                                seed_offset: sac_len + 6,
                                                tick_pct: 100.0,
                                                trip_noun: trip_noun.to_string(),
                                            )
                                            <div
                                                class="cut-chips"
                                                role="group"
                                                aria-label=(format!("Cuts for {}", b_gpt.noun))
                                            >
                                                <button
                                                    type="button"
                                                    class="cut-chip"
                                                    :aria-pressed=$(if p_gpt.get() == 1.0 { "true" } else { "false" })
                                                    @click=$(|_e| if p_gpt.get() == 1.0 { p_gpt.set(0.0) } else { p_gpt.set(1.0) })
                                                >(opt_label("chatgpt", "quit"))</button>
                                            </div>
                                        </div>
                                        <span class="row-equals">
                                            <strong>(format_bar_value(total_kg(b_gpt)))</strong>
                                        </span>
                                    </div>

                                    <div class="effort-row">
                                        row_head(
                                            noun: b_straw.noun,
                                            detail: b_straw.detail,
                                            source_ids: b_straw.source_ids.to_vec(),
                                        )
                                        <div class=(labeled_cell)>
                                            bar_track(
                                                bar_id: "straws",
                                                flight_kg: flight_kg,
                                                denom_kg: zoom_kg,
                                                tonnes_tips: false,
                                                seed_offset: sac_len + 7,
                                                tick_pct: 100.0,
                                                trip_noun: trip_noun.to_string(),
                                            )
                                            <div
                                                class="cut-chips"
                                                role="group"
                                                aria-label=(format!("Cuts for {}", b_straw.noun))
                                            >
                                                <button
                                                    type="button"
                                                    class="cut-chip"
                                                    :aria-pressed=$(if p_straw.get() == 1.0 { "true" } else { "false" })
                                                    @click=$(|_e| if p_straw.get() == 1.0 { p_straw.set(0.0) } else { p_straw.set(1.0) })
                                                >(opt_label("straws", "quit"))</button>
                                            </div>
                                        </div>
                                        <span class="row-equals">
                                            <strong>(format_bar_value(total_kg(b_straw)))</strong>
                                        </span>
                                    </div>
                                </div>
                                <hr class="flight-row-divider">

                                <div class="effort-row">
                                    <span class="row-head">
                                        "Your swaps, combined"
                                        <span class="row-detail">
                                            "tap the cuts above to fill this bar — or "
                                            <button
                                                type="button"
                                                class="link-btn"
                                                @click=$(|_e| {
                                                    p_heat.set(2.0);
                                                    p_cool.set(1.0);
                                                    p_diet.set(2.0);
                                                    p_waste.set(2.0);
                                                    p_fashion.set(2.0);
                                                    p_coffee.set(2.0);
                                                    p_phone.set(1.0);
                                                    p_soda.set(1.0);
                                                    p_bottle.set(1.0);
                                                    p_stream.set(1.0);
                                                    p_gpt.set(1.0);
                                                    p_straw.set(1.0);
                                                })
                                            >"take the whole monk year"</button>
                                        </span>
                                    </span>
                                    combined_swaps(
                                        flight_kg: $(fkg.get()),
                                        heat: $(p_heat.get()),
                                        cool: $(p_cool.get()),
                                        diet: $(p_diet.get()),
                                        waste: $(p_waste.get()),
                                        fashion: $(p_fashion.get()),
                                        coffee: $(p_coffee.get()),
                                        habits: $(p_phone.get()
                                            + p_soda.get() * 10.0
                                            + p_bottle.get() * 100.0
                                            + p_stream.get() * 1000.0
                                            + p_gpt.get() * 10000.0
                                            + p_straw.get() * 100000.0),
                                    )
                                </div>
                            </div>

                            <p class="chart-note caveat">"
                                Caveat: these bars measure greenhouse gases and nothing else.
                                Land use, water use, and plastic in the ocean are real problems in their own right.
                                A straw rounds to zero carbon and can still wash up on a beach or
                                kill a turtle. These issues deserve pages of their own. This page measures the
                                 one problem where flying towers over everything.
                            "</p>
                            cuts_data_tables(
                                flight_tonnes: flight_tonnes,
                                trip_noun: trip_noun.to_string(),
                            )
                        </div>
                    </div>

                    <div :hidden=$(view.get() != "compare")>
                        <div class="chart-card">
                            <div class="chart-card-head">
                                <p class="toggle-note">
                                    "each row picks units that fit this scale — tap a chip to \
                                     see what would make up for it"
                                </p>
                                <div class="chart-card-controls">
                                    <div class="makeup-chip" role="group" aria-label="Make up for it">
                                        <button
                                            type="button"
                                            :class=$(if m_transport.get() == "drive-less" { "makeup-chip-btn is-active" } else { "makeup-chip-btn" })
                                            :aria-pressed=$(if m_transport.get() == "drive-less" { "true" } else { "false" })
                                            @click=$(|_e| if m_transport.get() == "drive-less" { m_transport.set("".to_owned()) } else { m_transport.set("drive-less".to_owned()) })
                                        >"Drive less"</button>
                                        <button
                                            type="button"
                                            :class=$(if m_transport.get() == "go-ev" { "makeup-chip-btn is-active" } else { "makeup-chip-btn" })
                                            :aria-pressed=$(if m_transport.get() == "go-ev" { "true" } else { "false" })
                                            @click=$(|_e| if m_transport.get() == "go-ev" { m_transport.set("".to_owned()) } else { m_transport.set("go-ev".to_owned()) })
                                        >"Go EV"</button>
                                        <button
                                            type="button"
                                            :class=$(if m_food.get() == "eat-plants" { "makeup-chip-btn is-active" } else { "makeup-chip-btn" })
                                            :aria-pressed=$(if m_food.get() == "eat-plants" { "true" } else { "false" })
                                            @click=$(|_e| if m_food.get() == "eat-plants" { m_food.set("".to_owned()) } else { m_food.set("eat-plants".to_owned()) })
                                        >"Eat plants"</button>
                                        <button
                                            type="button"
                                            :class=$(if m_home.get() == "sweat-it-out" { "makeup-chip-btn is-active" } else { "makeup-chip-btn" })
                                            :aria-pressed=$(if m_home.get() == "sweat-it-out" { "true" } else { "false" })
                                            @click=$(|_e| if m_home.get() == "sweat-it-out" { m_home.set("".to_owned()) } else { m_home.set("sweat-it-out".to_owned()) })
                                        >"Sweat it out"</button>
                                        <button
                                            type="button"
                                            :class=$(if m_habits.get() == "skip-soda" { "makeup-chip-btn is-active" } else { "makeup-chip-btn" })
                                            :aria-pressed=$(if m_habits.get() == "skip-soda" { "true" } else { "false" })
                                            @click=$(|_e| if m_habits.get() == "skip-soda" { m_habits.set("".to_owned()) } else { m_habits.set("skip-soda".to_owned()) })
                                        >"Skip the soda"</button>
                                    </div>
                                </div>
                            </div>

                            <div class="effort-rows">
                                if flight_tonnes > 0.0 {
                                    <div class="effort-row">
                                        <span class="row-head">
                                            <strong>"This flight"</strong>
                                            <span class="row-detail">
                                                (format!("one {trip_noun} to {to_city}, one seat"))
                                                cite(id: "myclimate")
                                                cite(id: "lee2021")
                                            </span>
                                        </span>
                                        <div class="bar-cell">
                                            <div class="bar-h-track">
                                                <div
                                                    class="bar-seg"
                                                    tabindex="0"
                                                    style="width:100%;background:var(--cost)"
                                                >
                                                    <span class="tip">(format!("{} CO2e", format_bar_value(flight_kg)))</span>
                                                </div>
                                            </div>
                                        </div>
                                        <span class="row-equals">
                                            <strong>(format_bar_value(flight_kg))</strong>
                                        </span>
                                    </div>
                                    <hr class="flight-row-divider">
                                }
                                compare_domain_rows(
                                    scale_kg: $(fkg.get()),
                                    transport: $(m_transport.get()),
                                    food: $(m_food.get()),
                                    home: $(m_home.get()),
                                    habits: $(m_habits.get()),
                                )
                            </div>
                        </div>
                    </div>

                    <div :hidden=$(view.get() != "receipt")>
                        receipt::receipt(
                            from: from.clone(),
                            vias: vias.clone(),
                            to: to.clone(),
                            cabin: cabin,
                            round_trip: round_trip,
                            impact: impact,
                            share_path: share_path,
                        )
                    </div>

                    <div :hidden=$(view.get() != "allowance")>
                        budget_chart(
                            flight_tonnes: flight_tonnes,
                            economy_years: cabin_years(Cabin::Economy),
                            business_years: cabin_years(Cabin::Business),
                            first_years: cabin_years(Cabin::First),
                        )
                    </div>
                </div>
                if show_rail {
                    <div class="instrument-rail" :hidden=$(
                        if view.get() == "receipt" {
                            true
                        } else {
                            view.get() == "allowance"
                        }
                    )>
                        if impact.distance_km > 0.0 {
                            fuel_figure(
                                litres: fuel_litres,
                                litres_label: format_litres(fuel_litres),
                            )
                            seat_figure(
                                cabin: cabin,
                                seat_fraction: seat_fraction,
                                long_haul: long_haul,
                            )
                            driving_figure(flight_kg: flight_kg)
                        }
                        if show_ice {
                            ice_callout(ice_m2: impact.ice_m2)
                        }
                    </div>
                }
            </div>
        </section>
    }
}

/// "The flight vs. your travel allowance": the Paris aside, the arithmetic
/// walk-through, and the budget columns. `id="allowance"` is the anchor the
/// receipt's travel-allowance line links to.
#[component]
pub async fn budget_chart(
    flight_tonnes: f64,
    economy_years: f64,
    business_years: f64,
    first_years: f64,
) -> Result {
    const PLOT_HEIGHT: f64 = 190.0;
    let scale_max = BUDGET_TARGETS
        .iter()
        .map(|t| t.total)
        .fold(flight_tonnes, f64::max);
    let px = |t: f64| ((t / scale_max) * PLOT_HEIGHT).max(2.0);

    view! {
        <div class="allowance-panel" id="allowance">
            <aside class="aside-card">
                <h3>"The Paris Agreement, in sixty seconds"</h3>
                <p>
                    <strong>"What it is."</strong>
                    " A 2015 treaty adopted by 196 parties — nearly every country on Earth — to \
                     hold warming “well below 2 °C” and pursue 1.5 °C. "
                    cite(id: "paris")
                </p>
                <p>
                    <strong>"Is it too strict?"</strong>
                    " The people who run the numbers argue the opposite. In a 2024 survey of \
                     senior IPCC scientists, only 6% expected the 1.5 °C limit to hold, and \
                     nearly 80% foresee at least 2.5 °C. "
                    cite(id: "parisview")
                    " Among climate scientists, Paris isn’t ambitious — it’s the floor."
                </p>
                <p>
                    <strong>"Why back it anyway?"</strong>
                    " It’s the one plan everyone signed, it ratchets every five years, and the \
                     science beneath it — that burning carbon warms the planet — carries 97%+ \
                     agreement among publishing climate scientists. "
                    cite(id: "consensus")
                    " Every fraction of a degree it saves is suffering avoided."
                </p>
            </aside>
            <p class="section-sub">
                "The receipt ends with a line called “travel allowance used.” No treaty prints \
                 that number: researchers took the Paris Agreement’s ceiling, divided it into \
                 equal per-person shares, and asked how much of a share travel could claim. \
                 Equal shares is one defensible choice among several — here’s the arithmetic, \
                 so you can judge it."
            </p>
            <p class="section-sub">
                "The treaty itself (the card at right has the sixty-second version) binds \
                 countries, not people — it never mentions your vacation. Its core promise is a \
                 ceiling: hold the rise in global average temperature to well below 2 °C above \
                 pre-industrial levels, and pursue efforts to stop near 1.5 °C, the range past \
                 which heat, harvests, and coastlines get much harder to live with."
            </p>
            <p class="section-sub">
                "A temperature ceiling is secretly a carbon ceiling: past a certain total of \
                 CO₂e, the thermometer follows, no matter who emitted it. Researchers at Aalto \
                 University and Japan’s IGES turned that global total into a personal one — "
                <em>"if"</em>
                " you accept an equal per-person share (their arithmetic, not the treaty’s), a \
                 1.5 °C-compatible life can spend roughly 2.5 t a year by 2030, falling to \
                 0.7 t by 2050, for everything combined. "
                cite(id: "budgets")
                " The slice available for getting around — car, bus, train, ferry, and plane, \
                 all of it — is the travel allowance on your receipt: about 0.43 t a year at \
                 the 2030 milestone, tightening each decade after. Here, those allowances \
                 stand next to this one flight."
            </p>
            <div class="chart-card">
                <div class="budget-plot" style=(format!("height:{PLOT_HEIGHT}px"))>
                    <div class="budget-col">
                        <div class="col-value">(format_tonnes(flight_tonnes))</div>
                        <div
                            class="seg top"
                            tabindex="0"
                            style=(format!("height:{:.1}px;background:var(--cost)", px(flight_tonnes)))
                        >
                            <span class="tip">(format!(
                                "This flight: {} CO₂e",
                                format_tonnes(flight_tonnes)
                            ))</span>
                        </div>
                    </div>
                    for target in BUDGET_TARGETS.iter() {
                        <div class="budget-col">
                            <div class="col-value">(format_tonnes(target.total))</div>
                            <div
                                class="seg top"
                                tabindex="0"
                                style=(format!(
                                    "height:{:.1}px;background:var(--save-soft)",
                                    px(target.total - target.travel)
                                ))
                            >
                                <span class="tip">(format!(
                                    "{}: {}/yr per person in total — food, housing, goods, services",
                                    target.year,
                                    format_tonnes(target.total)
                                ))</span>
                            </div>
                            <div
                                class="seg"
                                tabindex="0"
                                style=(format!(
                                    "height:{:.1}px;background:var(--save)",
                                    px(target.travel)
                                ))
                            >
                                <span class="tip">(format!(
                                    "{}: {}/yr for all travel — car, bus, train, and plane combined",
                                    target.year,
                                    format_tonnes(target.travel)
                                ))</span>
                            </div>
                        </div>
                    }
                </div>
                <div class="budget-labels">
                    <div class="col-label">"this flight"</div>
                    for target in BUDGET_TARGETS.iter() {
                        <div class="col-label">(target.year)</div>
                    }
                </div>
                <dl class="benchmark-legend">
                    <div>
                        <dt>
                            <span class="swatch" style="background:var(--cost)"></span>
                            "this flight"
                        </dt>
                        <dd>"one itinerary, one seat — for comparison against whole years"</dd>
                    </div>
                    for target in BUDGET_TARGETS.iter() {
                        <div>
                            <dt>(target.year)</dt>
                            <dd>(format!(
                                "{} per person-year: {}",
                                format_tonnes(target.total),
                                target.meaning
                            ))</dd>
                        </div>
                    }
                    <div>
                        <dt>
                            <span class="swatch" style="background:var(--save)"></span>
                            "dark slice"
                        </dt>
                        <dd>
                            "the travel allowance: a full year of "
                            <em>"all"</em>
                            " mobility — every car trip, bus, train, ferry, and flight"
                        </dd>
                    </div>
                    <div>
                        <dt>
                            <span class="swatch" style="background:var(--save-soft)"></span>
                            "light slice"
                        </dt>
                        <dd>"everything else a life emits: food, housing, goods, services"</dd>
                    </div>
                </dl>
                <p class="chart-note">(format!(
                    "Whether or not you accept the budget, the levers are yours. This exact \
                     route runs ≈{} allowance-years in economy, ≈{} in business, and ≈{} in \
                     first — and the biggest lever of all is simply one trip fewer.",
                    format_years_span(economy_years),
                    format_years_span(business_years),
                    format_years_span(first_years)
                ))</p>
                <details class="data-table">
                    <summary>"Data table"</summary>
                    <table>
                        <thead>
                            <tr>
                                <th>"Bar"</th>
                                <th>"Total tCO₂e/yr"</th>
                                <th>"Travel share"</th>
                            </tr>
                        </thead>
                        <tbody>
                            <tr>
                                <td>"This flight (one-off)"</td>
                                <td>(format!("{flight_tonnes:.2}"))</td>
                                <td>"—"</td>
                            </tr>
                            for target in BUDGET_TARGETS.iter() {
                                <tr>
                                    <td>(format!("{} target", target.year))</td>
                                    <td>(format!("{:.2}", target.total))</td>
                                    <td>(format!("{:.2}", target.travel))</td>
                                </tr>
                            }
                        </tbody>
                    </table>
                </details>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The stylesheet hardcodes generated class names; these tests are the
    /// tripwire for the reference_data ↔ charts ↔ CSS coupling triangle.
    const CSS: &str = include_str!("../../../../styles/planes-charts.css");
    /// This file's own source: the `:data-pick-*` attributes declared in
    /// charts_section are the Rust side of the CSS attribute selectors.
    const SELF: &str = include_str!("charts.rs");

    /// Every `<stem><ident-chars>` token following an occurrence of
    /// `prefix` (whose leading punctuation is not part of the token). Bare
    /// stems (comments like "data-pick-*") are dropped.
    fn tokens_after(text: &str, prefix: &str) -> HashSet<String> {
        let stem = prefix.trim_start_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-'));
        text.match_indices(prefix)
            .map(|(idx, m)| {
                let rest: String = text[idx + m.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                format!("{stem}{rest}")
            })
            .filter(|token| token.len() > stem.len())
            .collect()
    }

    #[test]
    fn css_seg_classes_all_exist_in_reference_data() {
        let valid: HashSet<String> = all_bars()
            .flat_map(|bar| {
                bar.slices
                    .iter()
                    .map(move |slice| format!("seg-{}-{}", bar.id, slice.id))
            })
            .collect();
        let used = tokens_after(CSS, ".seg-");
        assert!(!used.is_empty(), "extraction found no .seg-* selectors");
        for class in used {
            assert!(
                valid.contains(&class),
                "planes-charts.css selects .{class}, but no bar/slice in reference_data.rs \
                 generates it — renamed id?"
            );
        }
    }

    #[test]
    fn css_data_pick_attrs_all_declared_in_charts() {
        let declared = tokens_after(SELF, ":data-pick-");
        let used = tokens_after(CSS, "data-pick-");
        assert!(!used.is_empty(), "extraction found no data-pick selectors");
        for token in used {
            assert!(
                declared.contains(&token),
                "planes-charts.css keys on [{token}] but charts_section declares no :{token} \
                 attribute"
            );
        }
    }

    /// The other direction: the tests above only prove CSS names exist in
    /// Rust, so a stylesheet rewrite could silently drop the whole erased
    /// table and still pass. Pin its shape — every declared pick attribute
    /// must appear in at least one CSS selector, and the selector table must
    /// keep its 22 entries (one per pick level × slice combination).
    #[test]
    fn css_erased_table_is_complete() {
        let declared = tokens_after(SELF, ":data-pick-");
        let used = tokens_after(CSS, "data-pick-");
        for token in &declared {
            assert!(
                used.contains(token),
                "charts_section declares :{token} but planes-charts.css never keys on it — \
                 erased styling silently lost?"
            );
        }
        let table_rows = CSS
            .lines()
            .filter(|l| l.contains("[data-pick-") && l.contains("] .seg-"))
            .count();
        assert!(
            table_rows >= 22,
            "the erased-slice selector table shrank to {table_rows} rows (expected ≥22) — \
             a stylesheet rewrite dropped part of it"
        );
    }
}

#[cfg(test)]
mod price_tests {
    use super::*;

    /// Chip labels hand-type their price ("go vegan −1.0 t") while the bar
    /// math computes the same saving from slice kg via `option_kg`. The two
    /// have drifted before; assert the displayed price rounds from the
    /// computed one (tolerance: half the label's last displayed digit).
    /// On short flights a habit bar outweighs the zoom track; widths must
    /// truncate at 100%, never rescale (the flex-shrink lie this replaced).
    #[test]
    fn clamped_widths_truncate_overflow() {
        let w = clamped_widths([60.0, 60.0, 60.0].into_iter(), 100.0);
        assert_eq!(w, vec![60.0, 40.0, 0.0]);
        let w = clamped_widths([25.0, 25.0].into_iter(), 100.0);
        assert_eq!(w, vec![25.0, 25.0]);
    }

    #[test]
    fn chip_price_labels_match_computed_savings() {
        for bar in all_bars() {
            for opt in bar.options {
                let Some((_, price)) = opt.label.rsplit_once(" \u{2212}") else {
                    panic!("{}:{} label has no \u{2212}price suffix", bar.id, opt.id);
                };
                let (num, unit) = price
                    .rsplit_once(' ')
                    .unwrap_or_else(|| panic!("{}:{} price {price:?}", bar.id, opt.id));
                let shown: f64 = num
                    .parse()
                    .unwrap_or_else(|_| panic!("{}:{} price number {num:?}", bar.id, opt.id));
                let actual_kg = option_kg(bar, opt.id);
                let actual = match unit {
                    "t" => actual_kg / 1000.0,
                    "kg" => actual_kg,
                    other => panic!("{}:{} price unit {other:?}", bar.id, opt.id),
                };
                let decimals = num.split('.').nth(1).map_or(0, str::len) as i32;
                let tol = 0.5 * 10f64.powi(-decimals) + 1e-9;
                assert!(
                    (shown - actual).abs() <= tol,
                    "{}:{} label says \u{2212}{num} {unit} but slices sum to {actual:.3} {unit}",
                    bar.id,
                    opt.id
                );
            }
        }
    }
}
