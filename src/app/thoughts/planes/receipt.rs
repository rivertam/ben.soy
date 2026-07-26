//! The flight climate receipt: printer chrome, itemized CO₂e bill, sea-ice
//! figure, QR share code and the comparison coupon. Ported from
//! `~/how-bad/src/components/{Receipt,IceGraphic,ReceiptComparisons}.tsx`.
//! The original's copy-for-friend / copy-as-image buttons are dropped per the
//! design doc; the shareable URL + QR carry that job.

use topcoat::{
    Result,
    context::Cx,
    router::{header, headers},
    view::{Unescaped, View, component, view},
};

use super::{
    airports::Airport,
    comparison_scale::{PickDomainRowsOptions, pick_domain_rows},
    emissions::{Cabin, FlightImpact, JET_FUEL_KG_PER_LITRE},
    format::{
        format_bar_value, format_ice, format_km, format_litres, format_tonnes, format_tonnes_smart,
        format_whole, format_years,
    },
    ice::{ICE_SHOW_FLOOR_M2, ice_figure},
    sources::cite,
};
use crate::components::link_label;

const DOT_LEADER: &str =
    "................................................................................";

fn sky_note(factor: f64) -> String {
    let f = format!("×{factor:.1}");
    let skies = if factor >= 1.25 {
        format!(
            "heat-trapping cloud trails, priced to contrail-prone skies ({f} the global average, \
             per km flown)"
        )
    } else if factor <= 0.75 {
        format!(
            "heat-trapping cloud trails, priced to contrail-lean skies ({f} the global average, \
             per km flown)"
        )
    } else {
        format!("heat-trapping cloud trails, priced to about-average skies ({f})")
    };
    format!(
        "{skies}. Weather decides the day — ≈2.7% of flights cause 80% of contrail warming; \
         science’s range on this line runs ⅓× to 1.7×"
    )
}

/// The share URL as a print-style QR SVG (margin 0; sized by CSS like the
/// original's `qrcode` npm output). Trusted markup: we generate it.
fn qr_svg(url: &str) -> String {
    let Ok(code) = qrcode::QrCode::new(url.as_bytes()) else {
        return String::new();
    };
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .quiet_zone(false)
        .build();
    // Strip the XML prolog; this is inline HTML, not a standalone document.
    match svg.find("?>") {
        Some(i) => svg[i + 2..].to_string(),
        None => svg,
    }
}

#[component]
async fn row(
    label: &str,
    footnotes: Vec<&'static str>,
    value: View,
    hero: bool,
    fine: bool,
) -> Result {
    view! {
        <div class=(if hero {
            "receipt-row hero"
        } else if fine {
            "receipt-row fine"
        } else {
            "receipt-row"
        })>
            <span class="r-label">
                (label)
                " "
                for id in footnotes {
                    cite(id: id)
                }
            </span>
            <span class="r-dots" aria-hidden="true">(DOT_LEADER)</span>
            <span class="r-value">(value)</span>
        </div>
    }
}

/// CVS-coupon zone printed at the foot of the receipt: the same
/// full-richness comparison rows as `ComparisonScale`, so a printed
/// receipt still carries the "compared to everything else" payoff.
#[component]
async fn receipt_comparisons(scale_kg: f64, compare_href: String) -> Result {
    let scale = if scale_kg > 0.0 { scale_kg } else { 1.0 };
    let rows = pick_domain_rows(scale, &PickDomainRowsOptions::default());

    view! {
        <div class="receipt-coupon">
            <div class="receipt-coupon-head">"★ SAVE THESE COMPARISONS ★"</div>
            <div class="receipt-items">
                for r in rows {
                    <div class="receipt-row fine">
                        <span class="r-label">(r.label)</span>
                        <span class="r-dots" aria-hidden="true">(DOT_LEADER)</span>
                        <span class="r-value">(format_bar_value(r.bar_fill_kg))</span>
                    </div>
                }
            </div>
            <p class="receipt-coupon-foot">
                <a href=(compare_href)>link_label(label: "see interactive scale →")</a>
            </p>
        </div>
    }
}

#[component]
pub async fn receipt(
    cx: &Cx,
    from: Airport,
    vias: Vec<Airport>,
    to: Airport,
    cabin: Cabin,
    round_trip: bool,
    impact: FlightImpact,
    share_path: String,
) -> Result {
    let staycation = impact.distance_km == 0.0;
    let legs_km = impact.distance_km * if round_trip { 2.0 } else { 1.0 };
    let seat_fraction = if staycation {
        1.0
    } else {
        (1.0 / impact.seat_share_of_aircraft).round()
    };
    let arrow = if round_trip { "⇄" } else { "→" };
    let show_ice = impact.ice_m2 >= ICE_SHOW_FLOOR_M2;

    // The QR encodes an absolute URL when the request names a host (the
    // public origin is whatever the visitor is browsing), the bare share
    // path otherwise.
    let hdrs = headers(cx);
    let qr_url = match hdrs.get(header::HOST).and_then(|h| h.to_str().ok()) {
        Some(host) => {
            let scheme = hdrs
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(
                    if host.starts_with("localhost") || host.starts_with("127.") {
                        "http"
                    } else {
                        "https"
                    },
                );
            format!("{scheme}://{host}{share_path}")
        }
        None => share_path.clone(),
    };
    let qr = qr_svg(&qr_url);

    // The interactive scale is this page's Compare view.
    let compare_href = format!(
        "{share_path}{}view=compare",
        if share_path.contains('?') { "&" } else { "?" }
    );
    let allowance_href = format!(
        "{share_path}{}view=allowance#allowance",
        if share_path.contains('?') { "&" } else { "?" }
    );

    // The itinerary chain, origin → layovers → destination; the round-trip
    // glyph joins every hop since each leg is flown both ways.
    let chain = std::iter::once(&from)
        .chain(vias.iter())
        .chain(std::iter::once(&to))
        .collect::<Vec<_>>();
    let route_line = chain
        .iter()
        .map(|a| format!("{} ({})", a.city, a.iata))
        .collect::<Vec<_>>()
        .join(&format!(" {arrow} "));
    let route_meta = if staycation {
        "you’re already there — 0 km flown".to_string()
    } else {
        format!(
            "1 ticket · {} · {} · {} flown",
            cabin.as_str(),
            if round_trip { "round trip" } else { "one way" },
            format_km(legs_km)
        )
    };
    let code_text = format!(
        "{} · {}",
        chain
            .iter()
            .map(|a| a.iata.as_str())
            .collect::<Vec<_>>()
            .join(&format!(" {arrow} ")),
        cabin.as_str().to_uppercase()
    );

    let whole_flight_value = view! { (format_tonnes(impact.aircraft_tonnes_co2e)) }?;
    let hero_value = view! { (format_tonnes(impact.tonnes_co2e)) }?;
    let fuel_value = view! {
        <span class="r-value-sub">
            (format!("{} ·", format_litres(impact.fuel_kg / JET_FUEL_KG_PER_LITRE)))
        </span>
        " "
        (format_tonnes_smart(impact.co2_tonnes))
    }?;
    let contrail_value = view! { (format_tonnes_smart(impact.contrail_tonnes)) }?;
    let nox_value = view! { (format_tonnes_smart(impact.nox_other_tonnes)) }?;
    let wtt_value = view! { (format_tonnes_smart(impact.wtt_tonnes)) }?;
    let ice_value = view! { (format_ice(impact.ice_m2)) }?;

    view! {
        <section class="printer" aria-label="Flight climate receipt">
            <div class="printer-slot">
                <span class="printer-led" aria-hidden="true"></span>
            </div>
            <div class="print-window">
                <div class="print-paper">
                    <div class="print-feed">
                        <div class="receipt">
                            <div class="receipt-head">
                                <div class="receipt-title">"Flight Receipt"</div>
                                <div class="receipt-tag">"per ticket · one seat's share of the flight"</div>
                            </div>
                            <div class="receipt-route">
                                <div class="route-line">(route_line)</div>
                                <div class="route-meta">(route_meta)</div>
                            </div>
                            <div class="receipt-items">
                                if !staycation {
                                    row(
                                        label: "Whole flight (aircraft)",
                                        footnotes: vec!["myclimate", "lee2021"],
                                        value: whole_flight_value,
                                        hero: false,
                                        fine: true,
                                    )
                                    <div class="receipt-allocation">
                                        (format!(
                                            "Your {} ticket ≈ 1/{} of the aircraft — the plane's full impact, split by seat\u{a0}",
                                            cabin.as_str(),
                                            format_whole(seat_fraction)
                                        ))
                                        cite(id: "myclimate")
                                    </div>
                                }
                                row(
                                    label: if staycation { "CO₂ equivalent" } else { "CO₂ equivalent · your ticket" },
                                    footnotes: vec!["myclimate", "lee2021"],
                                    value: hero_value,
                                    hero: true,
                                    fine: false,
                                )
                                if !staycation {
                                    <div class="co2-note">
                                        "itemized below"
                                        cite(id: "defra2025")
                                    </div>
                                    row(
                                        label: "Jet fuel burned",
                                        footnotes: vec!["myclimate", "jetfuel-density"],
                                        value: fuel_value,
                                        hero: false,
                                        fine: true,
                                    )
                                    row(
                                        label: "Contrail cirrus, expected",
                                        footnotes: vec!["lee2021", "teoh2024"],
                                        value: contrail_value,
                                        hero: false,
                                        fine: true,
                                    )
                                    <div class="co2-note">(sky_note(impact.sky_factor))</div>
                                    row(
                                        label: "NOx & other altitude effects",
                                        footnotes: vec!["lee2021"],
                                        value: nox_value,
                                        hero: false,
                                        fine: true,
                                    )
                                    row(
                                        label: "Making the fuel",
                                        footnotes: vec!["myclimate"],
                                        value: wtt_value,
                                        hero: false,
                                        fine: true,
                                    )
                                }
                                if show_ice {
                                    row(
                                        label: "Arctic sea ice melted",
                                        footnotes: vec!["seaice"],
                                        value: ice_value,
                                        hero: false,
                                        fine: false,
                                    )
                                    if !staycation {
                                        <div class="co2-note">
                                            "3 m² per tonne of CO₂, the jet-fuel line only — decades of satellite ice maps plotted against humanity's cumulative CO₂ fall on a straight line"
                                        </div>
                                    }
                                    ice_figure(ice_m2: impact.ice_m2, pattern_id: "ice-dots-pat")
                                }
                            </div>
                            <div class="receipt-total">
                                <div class="receipt-row">
                                    <span class="r-label">
                                        "Travel allowance used "
                                        cite(id: "budgets")
                                    </span>
                                    <span class="r-dots" aria-hidden="true">(DOT_LEADER)</span>
                                    <span class="r-value">(format_years(impact.travel_budget_years))</span>
                                </div>
                                <span class="note">
                                    "years of the ≈0.43 t/yr the 2030 target leaves one person for "
                                    <em>"all"</em>
                                    " travel on a 1.5 °C path — "
                                    <a href=(allowance_href)>"see Allowance tab →"</a>
                                </span>
                            </div>
                            <div class="receipt-code">
                                <div class="qr" aria-hidden="true">
                                    (Unescaped::new_unchecked(qr))
                                </div>
                                <div class="code-text">(code_text)</div>
                            </div>
                            if staycation {
                                <div class="receipt-foot">"*** NO CHARGE — ENJOY THE STAYCATION ***"</div>
                            } else {
                                receipt_comparisons(
                                    scale_kg: impact.tonnes_co2e * 1000.0,
                                    compare_href: compare_href,
                                )
                            }
                        </div>
                    </div>
                </div>
            </div>
        </section>
    }
}
