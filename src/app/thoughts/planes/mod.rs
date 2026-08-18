//! The flight page as a dispatch desk: a full-bleed working surface where
//! the route form is a flight-progress strip, the charts are the climate
//! paperwork the desk returns, and the essay rides alongside as the
//! dispatcher's margin notes. The page-level composition still mirrors the
//! original `App.tsx` data flow: resolve the route from the query string,
//! compute the impact server-side, and render form → cuts → sources. URLs stay
//! shareable exactly like the original.

mod airports;
mod charts;
mod emissions;
mod form;
mod format;
mod ice;
mod instruments;
mod reference_data;
mod share;
mod sources;

crate::register_post!(
    essay,
    slug: "how-bad-are-planes",
    title: "How bad are planes?",
    date: "2026-07-12",
    teaser: "Why I don't generally take planes for leisure",
    tags: &["climate", "planes"],
);

use topcoat::{
    Result,
    context::Cx,
    router::{page, query_params, uri},
    view::view,
};

use crate::components::{doc_head, ext_link, full_bleed, inline_popover, margin_notes, shell};

use self::{
    airports::{Airport, find_airport},
    emissions::{Cabin, Coordinates, route_impact},
    format::{format_km, format_tonnes},
    sources::sources_section,
};

use form::flight_form;

// Layovers deliberately do NOT appear here: `#[query_params]` parses with
// serde_urlencoded, where a repeated *declared* field is a duplicate-field
// error (and the error redirect would wipe the whole query string), while
// repeated keys the struct doesn't declare are simply ignored. So `via`
// repeats freely in the URL and `parse_vias` reads it from the raw query.
#[query_params(error = redirect("?"))]
struct PlanesQuery {
    from: Option<String>,
    to: Option<String>,
    cabin: Option<String>,
    oneway: Option<String>,
    trip: Option<String>,
}

fn resolve(param: Option<&str>) -> Option<&'static Airport> {
    find_airport(param?.trim())
}

/// The route's layovers, in flight order: every repeated `via` param in the
/// raw query string (numbered `via1`..`viaN` names parse too — an earlier
/// revision of this page emitted them). Unresolvable values drop out
/// (matching `from`/`to`, whose unresolved text also doesn't survive the
/// round trip); the route falls back to fewer stops rather than
/// un-revealing.
fn parse_vias(query: &str) -> Vec<&'static Airport> {
    form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| {
            key.strip_prefix("via")
                .is_some_and(|rest| rest.is_empty() || rest.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|(_, value)| find_airport(value.trim()))
        .collect()
}

/// The share query mirrors the original's `routeSearchParams`: defaults
/// (economy, round trip) are omitted, and layovers ride as repeated `via`
/// params in flight order. `iatas` is the whole chain — origin, layovers,
/// destination.
fn share_path(iatas: &[&str], cabin: Cabin, round_trip: bool) -> String {
    let mut path = format!(
        "/thoughts/how-bad-are-planes?from={}&to={}",
        iatas.first().unwrap_or(&""),
        iatas.last().unwrap_or(&"")
    );
    if iatas.len() > 2 {
        for via in &iatas[1..iatas.len() - 1] {
            path.push_str(&format!("&via={via}"));
        }
    }
    if cabin != Cabin::Economy {
        path.push_str(&format!("&cabin={}", cabin.as_str()));
    }
    if !round_trip {
        path.push_str("&trip=oneway");
    }
    path
}

#[page("/thoughts/how-bad-are-planes")]
async fn planes(cx: &Cx) -> Result {
    let q = query_params::<PlanesQuery>(cx)?;

    let from = resolve(q.from.as_deref());
    let to = resolve(q.to.as_deref());
    let vias = parse_vias(uri(cx).query().unwrap_or(""));
    let cabin = q
        .cabin
        .as_deref()
        .and_then(Cabin::parse)
        .unwrap_or(Cabin::Economy);
    // The original marked one-way with the mere presence of `oneway` (any
    // value); the form's trip radios say `trip=oneway`. Accept both so old
    // share URLs keep working.
    let round_trip = !(q.oneway.is_some() || q.trip.as_deref() == Some("oneway"));
    let revealed = from.is_some() && to.is_some();

    let title;
    let mut seal_total = String::new();
    // The vias the form redisplays: the canonical collapsed chain once the
    // route is filed, the raw resolved codes before that.
    let mut form_vias: Vec<Airport> = vias.iter().map(|a| (*a).clone()).collect();
    // The route card rides the top of the margin column, above the notes —
    // outside the dispatch pane, so it's built alongside revealed_part.
    let mut route_card = None;
    let revealed_part = match (from, to) {
        (Some(from), Some(to)) => {
            // Origin → layovers → destination, with a via collapsed when it
            // repeats its neighbor (it would only add a zero-length leg and
            // a silly route line). from == to with no vias stays a two-stop
            // staycation, which is why only vias collapse — never `to`.
            let mut chain: Vec<&'static Airport> = vec![from];
            for via in &vias {
                if via.iata != chain.last().expect("chain has origin").iata {
                    chain.push(via);
                }
            }
            if chain.len() > 1 && chain.last().expect("chain has stops").iata == to.iata {
                chain.pop();
            }
            chain.push(to);
            let route_vias: Vec<Airport> = chain[1..chain.len() - 1]
                .iter()
                .map(|a| (*a).clone())
                .collect();
            form_vias = route_vias.clone();

            let stops: Vec<Coordinates> = chain.iter().map(|a| a.coordinates()).collect();
            let impact = route_impact(&stops, cabin, round_trip);
            let legs_km = impact.distance_km * if round_trip { 2.0 } else { 1.0 };
            let figure_stops: Vec<Airport> = chain.iter().map(|a| (*a).clone()).collect();
            route_card = Some(view! {
                <div class="dispatch desk-route">
                    instruments::route_figure(
                        stops: figure_stops,
                        round_trip: round_trip,
                        km_flown: format_km(legs_km),
                    )
                </div>
            }?);

            let iatas: Vec<&str> = chain.iter().map(|a| a.iata.as_str()).collect();
            let share_path = share_path(&iatas, cabin, round_trip);
            let share_origin = share::request_origin(cx);
            let share_text = share::share_text(
                share_origin.as_deref(),
                from,
                &route_vias,
                to,
                cabin,
                round_trip,
                &impact,
                &share_path,
            );
            seal_total = format_tonnes(impact.tonnes_co2e);
            title = format!(
                "{} · {} CO₂e — {}",
                iatas.join(if round_trip { " ⇄ " } else { " → " }),
                format_tonnes(impact.tonnes_co2e),
                POST.title,
            );

            Some(view! {
                charts::charts_section(
                    impact: impact,
                    round_trip: round_trip,
                    from: from.clone(),
                    vias: route_vias,
                    to: to.clone(),
                    cabin: cabin,
                    share_text: share_text,
                )
                sources_section()
            }?)
        }
        _ => {
            title = POST.title.to_string();
            None
        }
    };

    let spread_class = if revealed {
        "desk-spread desk-spread--filed"
    } else {
        "desk-spread"
    };

    view! { shell(title: title.as_str(), active: "", hide_nav: true,
        <article>
            full_bleed(class: "desk-band",
                <div class=(spread_class)>
                    doc_head(stamp: POST.date, title: POST.title)
                    <div class="dispatch">
                        flight_form(
                            from: from.cloned(),
                            to: to.cloned(),
                            vias: form_vias,
                            cabin: cabin,
                            round_trip: round_trip,
                            revealed: revealed,
                            total: seal_total,
                        )
                        if !revealed {
                            <div class="strip-ghosts" aria-hidden="true">
                                <div></div>
                                <div></div>
                                <div></div>
                            </div>
                        }
                        if let Some(part) = revealed_part {
                            (part)
                        }
                    </div>
                    if let Some(card) = route_card {
                        (card)
                    }
                    margin_notes(stamp: "",
                        <p>
                            "In 2019, I read "
                            inline_popover(
                                id: "planet-b-cite",
                                label: "There Is No Planet B",
                                <span class="inline-popover-preview">
                                    "Mike Berners-Lee’s 2019 handbook on climate priorities — where flying \
                                     lands among the high-impact personal choices."
                                </span>
                                ext_link(
                                    class: "quiet-link",
                                    href: "https://theresnoplanetb.net/",
                                    label: "theresnoplanetb.net →"
                                )
                            )
                            " by Mike Berners-Lee a couple days before a trip I took to Asheville, \
                             North Carolina to visit my mom. I learned not just " <em>"that"</em>" planes are bad \
                             for the environment, but the magnitude."
                        </p>
                        <p>
                            "One of my favorite philosophies in life is the "
                            inline_popover(
                                id: "pareto-cite",
                                label: "Pareto Principle",
                                <span class="inline-popover-preview">
                                    "Also called the 80/20 rule: a small share of causes often drives \
                                     most of the effect. Named for Vilfredo Pareto’s observation about \
                                     wealth concentration."
                                </span>
                                ext_link(
                                    class: "quiet-link",
                                    href: "https://en.wikipedia.org/wiki/Pareto_principle",
                                    label: "Wikipedia →"
                                )
                            )
                            ": don't waste all your time and effort on the minutiae. Find the points \
                             of highest impact. What I discovered is that, among the people I know and \
                             myself historically, flying planes eclipses almost all of our other habits. \
                             I would say for most people I know, four domestic flights (round trip) \
                             each year is quite typical, with international trips at least once every \
                             2-3 years. I personally feel this calculator typically illustrates \
                             why I think most flights are simply not worth it."
                        </p>

                        <p>
                            "I also know people who have essentially never been on planes. Most of them \
                             simply can't afford it, often to the extent that they haven't even considered \
                             traveling for leisure."
                        </p>

                        <p>
                            "Commercial consumer flying accounts for "
                            inline_popover(
                                id: "aviation-share-cite",
                                label: "about 2% of the world's CO₂",
                                <span class="inline-popover-preview">
                                    "All aviation is ~2.5% of global CO₂ (fossil + land use). With \
                                     non-CO₂ effects — mainly contrail cirrus — it’s ~3.5% of warming \
                                     to date. Of that CO₂, ~88% is commercial, ~8% military, and ~4% \
                                     private; within commercial, ~81% is passengers and ~19% freight. \
                                     Passenger flying alone is therefore ~2% of global CO₂."
                                </span>
                                ext_link(
                                    class: "quiet-link",
                                    href: "https://ourworldindata.org/global-aviation-emissions",
                                    label: "Our World in Data →"
                                )
                                ext_link(
                                    class: "quiet-link",
                                    href: "https://doi.org/10.1016/j.gloenvcha.2020.102194",
                                    label: "Gössling & Humpe 2020 →"
                                )
                            )
                            " despite benefiting a "
                            inline_popover(
                                id: "who-flies-cite",
                                label: "sliver of the population",
                                <span class="inline-popover-preview">
                                    "Gössling & Humpe (2020): about 11% of the world flew in 2018, at \
                                     most 4% internationally. The most frequent 1% of people account \
                                     for more than half of passenger-aviation CO₂."
                                </span>
                                ext_link(
                                    class: "quiet-link",
                                    href: "https://doi.org/10.1016/j.gloenvcha.2020.102194",
                                    label: "Gössling & Humpe 2020 →"
                                )
                            )
                            ". A huge portion of the global south have either never flown on a plane \
                             or it's a very rare, very expensive privilege used for special circumstances \
                             such as migrating between countries. Flying and travel to this extent has \
                             only been in human lives for the last "
                            inline_popover(
                                id: "jet-age-cite",
                                label: "~80 years",
                                <span class="inline-popover-preview">
                                    "Mass commercial jet travel starts in the 1950s (Comet, then the \
                                     Boeing 707). Aviation CO₂ has roughly quadrupled since the \
                                     mid-1960s, and its share of global emissions is still rising."
                                </span>
                                ext_link(
                                    class: "quiet-link",
                                    href: "https://ourworldindata.org/global-aviation-emissions",
                                    label: "Our World in Data →"
                                )
                            )
                            ", and it's ramped up significantly and isn't stopping. Our
                             great-great-grandparents had to take ships to get across the Atlantic Ocean."
                        </p>

                        <p>"
                            Upsides exist: cultural diffusion, collaboration across borders, a
                             smaller world. If you want to argue that the pros of commercial travel outweigh the cons,
                             fine — but let's at least get the cons right first.
                        "</p>
                    )
                </div>
            )
        </article>
    ) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_path_keeps_the_original_nonstop_format() {
        // Old share URLs must keep round-tripping: defaults omitted, no via
        // params on a nonstop.
        assert_eq!(
            share_path(&["JFK", "LHR"], Cabin::Economy, true),
            "/thoughts/how-bad-are-planes?from=JFK&to=LHR"
        );
        assert_eq!(
            share_path(&["JFK", "LHR"], Cabin::Business, false),
            "/thoughts/how-bad-are-planes?from=JFK&to=LHR&cabin=business&trip=oneway"
        );
    }

    #[test]
    fn share_path_repeats_vias_in_flight_order() {
        assert_eq!(
            share_path(&["JFK", "KEF", "AMS", "LHR"], Cabin::Economy, true),
            "/thoughts/how-bad-are-planes?from=JFK&to=LHR&via=KEF&via=AMS"
        );
    }

    #[test]
    fn vias_parse_in_appearance_order_and_round_trip_the_share_path() {
        let iatas =
            |vias: Vec<&'static Airport>| vias.iter().map(|a| a.iata.as_str()).collect::<Vec<_>>();
        assert_eq!(
            iatas(parse_vias("from=JFK&to=LHR&via=KEF&via=AMS&trip=oneway")),
            ["KEF", "AMS"]
        );
        // The numbered names an earlier revision emitted still parse, and
        // unresolvable or empty values drop out.
        assert_eq!(
            iatas(parse_vias("via1=KEF&via=&via2=ZZZZ&via=ams")),
            ["KEF", "AMS"]
        );
        assert!(parse_vias("viable=KEF&victory=AMS").is_empty());
        assert!(parse_vias("").is_empty());
    }
}
