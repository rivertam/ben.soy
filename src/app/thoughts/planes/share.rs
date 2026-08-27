//! Copy/paste sharing for one flight: a compact plain-text summary followed
//! by the canonical route URL. The disclosure is always selectable; the small
//! client enhancement only adds a clipboard button, matching the lifting
//! page's no-JavaScript path.

use topcoat::{
    Result,
    asset::{Asset, asset},
    context::Cx,
    router::{header, request::headers},
    view::{component, view},
};

use super::{
    airports::Airport,
    emissions::{Cabin, FlightImpact, JET_FUEL_KG_PER_LITRE},
    format::{format_count, format_km, format_litres, format_tonnes, format_tonnes_smart},
    reference_data::{HABIT_BARS, SACRIFICE_BARS, pick_analogy},
};

pub(super) const SHARE_JS: Asset = asset!("./share.js");

const SHARE_SUMMARY: &str = "w-fit py-[0.2rem] list-none [&::-webkit-details-marker]:hidden \
     text-oxide font-meta text-[0.72rem] cursor-pointer select-none \
     underline decoration-oxide/45 underline-offset-[0.24em] \
     group-open:decoration-current \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";
const SHARE_TEXT: &str = "block w-full max-h-[16rem] p-3 overflow-auto resize-y \
     whitespace-pre bg-page border border-hairline rounded-none \
     font-meta text-[0.7rem] leading-[1.6] text-ink2 outline-none \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";
const SHARE_BUTTON: &str = "mt-2 px-3 py-[0.45rem] font-meta text-[0.7rem] text-card bg-oxide \
     border border-oxide rounded-[0.2rem] cursor-pointer hover:text-white hover:bg-oxide-hot \
     hover:border-oxide-hot focus-visible:text-white focus-visible:bg-oxide-hot \
     focus-visible:border-oxide-hot";
const SHARE_HINT: &str = "mt-2 font-meta text-[0.67rem] leading-[1.5] text-muted";

/// The absolute origin the visitor is browsing, or `None` when the request
/// names no host and the share text should keep a bare path.
pub(super) fn request_origin(cx: &Cx) -> Option<String> {
    let hdrs = headers(cx);
    let host = hdrs.get(header::HOST).and_then(|h| h.to_str().ok())?;
    let scheme = hdrs
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(
            if host.starts_with("localhost") || host.starts_with("127.") {
                "http"
            } else {
                "https"
            },
        );
    Some(format!("{scheme}://{host}"))
}

/// The first four comparison rows on the Cuts chart, using the same seeds as
/// their dashed-line labels. Keeping this selection here means a copied share
/// sheet and the page speak in the same comparisons for the same flight.
fn top_comparisons(flight_kg: f64) -> Vec<String> {
    SACRIFICE_BARS
        .iter()
        .chain(HABIT_BARS.iter())
        .take(4)
        .enumerate()
        .map(|(index, bar)| {
            let (analogy, count) = pick_analogy(bar.analogies, flight_kg, index as i64 + 1);
            analogy.tick.replace("{n}", &format_count(count))
        })
        .collect()
}

/// Plain-text flight summary: route, ticket-level impact, itemized climate
/// lines, and the permanent URL at the end for easy pasting into a message.
pub(super) fn share_text(
    origin: Option<&str>,
    from: &Airport,
    vias: &[Airport],
    to: &Airport,
    cabin: Cabin,
    round_trip: bool,
    impact: &FlightImpact,
    path: &str,
) -> String {
    let chain: Vec<&Airport> = std::iter::once(from)
        .chain(vias.iter())
        .chain(std::iter::once(to))
        .collect();
    let arrow = if round_trip { " ⇄ " } else { " → " };
    let route = chain
        .iter()
        .map(|airport| format!("{} ({})", airport.city, airport.iata))
        .collect::<Vec<_>>()
        .join(arrow);
    let legs_km = impact.distance_km * if round_trip { 2.0 } else { 1.0 };
    let trip = if round_trip { "round trip" } else { "one way" };

    let mut lines = vec![
        "How bad are planes?".to_string(),
        route,
        format!(
            "{} · {trip} · {} · {}",
            cabin.as_str(),
            format_km(legs_km),
            format_tonnes(impact.tonnes_co2e)
        ),
        format!("{} CO₂e for one seat", format_tonnes(impact.tonnes_co2e)),
    ];

    if impact.distance_km > 0.0 {
        lines.push(String::new());
        lines.push("Top comparisons:".to_string());
        lines.extend(top_comparisons(impact.tonnes_co2e * 1000.0));
        lines.push(String::new());
        lines.push(format!(
            "Jet fuel: {} · {} CO₂",
            format_litres(impact.fuel_kg / JET_FUEL_KG_PER_LITRE),
            format_tonnes_smart(impact.co2_tonnes),
        ));
        lines.push(format!(
            "Contrails (expected): {} CO₂e",
            format_tonnes_smart(impact.contrail_tonnes),
        ));
        lines.push(format!(
            "Other altitude effects: {} CO₂e",
            format_tonnes_smart(impact.nox_other_tonnes),
        ));
        lines.push(format!(
            "Making the fuel: {} CO₂e",
            format_tonnes_smart(impact.wtt_tonnes),
        ));
    }

    lines.push(String::new());
    lines.push(match origin {
        Some(origin) => format!("{origin}{path}"),
        None => path.to_string(),
    });
    lines.join("\n")
}

/// The disclosure a flight page renders. The textarea is the no-JavaScript
/// path; `share.js` progressively reveals the clipboard button.
#[component]
pub(super) async fn share_block(text: &str) -> Result {
    let rows = text.lines().count().clamp(3, 14).to_string();
    view! {
        <details class="group" data-share="">
            <summary class=(SHARE_SUMMARY)>"share this flight"</summary>
            <div class="mt-2 max-w-[34rem]">
                <textarea
                    class=(SHARE_TEXT)
                    readonly=""
                    rows=(rows.as_str())
                    wrap="off"
                    spellcheck="false"
                    aria-label="Flight share text"
                >(text)</textarea>
                <button class=(SHARE_BUTTON) type="button" data-share-copy="" hidden="">
                    "copy to clipboard"
                </button>
                <p class=(SHARE_HINT) data-share-hint="">
                    "Select the text above to copy it — it already ends with this flight's link."
                </p>
            </div>
        </details>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn airport(iata: &str, city: &str) -> Airport {
        Airport {
            iata: iata.to_string(),
            name: city.to_string(),
            city: city.to_string(),
            country: "Testland".to_string(),
            lat: 0.0,
            lon: 0.0,
            weight: 1,
        }
    }

    fn impact() -> FlightImpact {
        FlightImpact {
            distance_km: 1000.0,
            tonnes_co2e: 1.234,
            co2_tonnes: 0.5,
            fuel_kg: 200.0,
            wtt_tonnes: 0.1,
            contrail_tonnes: 0.4,
            nox_other_tonnes: 0.2,
            sky_factor: 1.0,
            seat_share_of_aircraft: 1.0 / 100.0,
            aircraft_tonnes_co2e: 123.4,
            ice_m2: 3.7,
        }
    }

    #[test]
    fn share_text_ends_with_the_absolute_route() {
        let from = airport("AAA", "Alpha");
        let via = airport("BBB", "Bravo");
        let to = airport("CCC", "Charlie");
        let text = share_text(
            Some("https://example.test"),
            &from,
            &[via],
            &to,
            Cabin::Business,
            true,
            &impact(),
            "/thoughts/how-bad-are-planes?from=AAA&to=CCC&via=BBB&cabin=business",
        );

        assert!(text.starts_with("How bad are planes?\nAlpha (AAA) ⇄ Bravo (BBB) ⇄ Charlie (CCC)"));
        assert!(text.contains("business · round trip · 2,000 km"));
        assert!(text.ends_with(
            "https://example.test/thoughts/how-bad-are-planes?from=AAA&to=CCC&via=BBB&cabin=business"
        ));
    }

    #[test]
    fn staycation_share_skips_fuel_breakdown() {
        let from = airport("AAA", "Alpha");
        let to = airport("AAA", "Alpha");
        let mut flight = impact();
        flight.distance_km = 0.0;
        let text = share_text(
            None,
            &from,
            &[],
            &to,
            Cabin::Economy,
            true,
            &flight,
            "/thoughts/how-bad-are-planes?from=AAA&to=AAA",
        );

        assert!(!text.contains("Jet fuel:"));
        assert!(!text.contains("Top comparisons:"));
        assert!(text.ends_with("/thoughts/how-bad-are-planes?from=AAA&to=AAA"));
    }

    #[test]
    fn share_text_includes_four_chart_comparisons() {
        let from = airport("AAA", "Alpha");
        let to = airport("BBB", "Bravo");
        let text = share_text(
            None,
            &from,
            &[],
            &to,
            Cabin::Economy,
            false,
            &impact(),
            "/thoughts/how-bad-are-planes?from=AAA&to=BBB&trip=oneway",
        );

        let start = text.find("Top comparisons:\n").unwrap();
        let end = text.find("\n\nJet fuel:").unwrap();
        assert_eq!(text[start..end].lines().count(), 5);
    }
}
