//! First-party, public-by-aggregate site analytics.

mod dashboard;
mod db;
pub(super) mod guard;
mod input;
mod map;

use std::time::{SystemTime, UNIX_EPOCH};

use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, HeaderValue, Response, StatusCode, header, headers, page, redirect, route, to_bytes,
        uri,
    },
    session,
    view::view,
};

use benjisponge::data::Data;

use self::{
    dashboard::{Cohort, Day, Window},
    guard::{AnalyticsGuard, WriteKind},
    input::{
        EventPayload, current_page_matches, is_canonical_v4_uuid, is_same_origin,
        tracking_opted_out,
    },
};
use crate::components::{page_head, rail_section, shell};

const EVENT_BODY_LIMIT: usize = 4 * 1024;
const IDENTITY_BODY_LIMIT: usize = 2 * 1024;
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const NO_STORE: &str = "no-store";

#[derive(Debug)]
struct IdentityInput {
    display_name: String,
    note: Option<String>,
    bootstrap_id: Option<String>,
    honeypot: bool,
}

#[derive(Debug)]
struct Trend {
    line: String,
    area: String,
    maximum: i64,
    first_date: String,
    last_date: String,
}

#[derive(Debug)]
struct MapMarker {
    code: String,
    name: &'static str,
    views: i64,
    visitors: i64,
    x: f64,
    y: f64,
    radius: f64,
}

#[page("/analytics")]
async fn analytics(cx: &Cx) -> Result {
    // Accept only canonical cache keys. Unknown query strings redirect before
    // touching the database, so nonce parameters cannot amplify dashboard work.
    let window = match uri(cx).query() {
        None => Window::Month,
        Some("range=7d") => Window::Week,
        Some("range=90d") => Window::Quarter,
        Some("range=365d") => Window::Year,
        _ => return Err(redirect("/analytics").into()),
    };
    let now = epoch_seconds();
    let today_utc = now - now.rem_euclid(86_400);
    let cutoff = today_utc.saturating_sub((window.days() - 1) * 86_400);

    let snapshot = match app_context::<Data>(cx).db().await {
        Ok(db) => match dashboard::load(&db, cutoff).await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                log_failure("/analytics", error);
                None
            }
        },
        Err(error) => {
            log_failure("/analytics", error);
            None
        }
    };

    let trend = snapshot
        .as_ref()
        .map(|dashboard| trend_geometry(&dashboard.days));
    let markers = snapshot
        .as_ref()
        .map(|dashboard| map_markers(&dashboard.countries))
        .unwrap_or_default();
    let heat_max = snapshot
        .as_ref()
        .and_then(|dashboard| dashboard.hourly.iter().flatten().max().copied())
        .unwrap_or(0)
        .max(1);

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(
            "public, max-age=0, s-maxage=60"
        )))
        shell(
            title: "Analytics",
            active: "",
            runtime: false,
            page_head(
                stamp: "open ledger",
                title: "Analytics",
                lede: "The site's instrument panel — public numbers, private people.",
            )

            <section class="analytics-range mt-7" aria-label="Analytics date range">
                <p class="analytics-range-label">"window"</p>
                <nav class="analytics-range-options">
                    for option in Window::ALL {
                        <a
                            href=(if option == Window::Month {
                                "/analytics".to_string()
                            } else {
                                format!("/analytics?range={}", option.slug())
                            })
                            aria-current=(if option == window { "page" } else { "false" })
                        >(option.slug())</a>
                    }
                </nav>
                <p class="analytics-range-note">
                    "rolling " (window.label()) " · UTC days · refreshed about once a minute"
                </p>
            </section>

            if let Some(data) = snapshot.as_ref() {
                rail_section(
                    class: "mt-10",
                    stamp: "readout",
                    <div class="analytics-metrics">
                        <article class="analytics-metric analytics-metric-primary">
                            <p class="analytics-metric-label">"page views"</p>
                            <p class="analytics-metric-value">(format_number(data.overview.pageviews))</p>
                            <p class="analytics-metric-note">
                                (pluralized(
                                    data.overview.sessions,
                                    "reading session",
                                    "reading sessions",
                                ))
                            </p>
                        </article>
                        <article class="analytics-metric">
                            <p class="analytics-metric-label">"visitors"</p>
                            <p class="analytics-metric-value">(format_number(data.overview.visitors))</p>
                            <p class="analytics-metric-note">
                                (format!("{}% came back", data.overview.returning_percent))
                            </p>
                        </article>
                        <article class="analytics-metric">
                            <p class="analytics-metric-label">"attention"</p>
                            <p class="analytics-metric-value">
                                (format_duration(data.overview.engaged_seconds))
                            </p>
                            <p class="analytics-metric-note">"visible reading time"</p>
                        </article>
                        <article class="analytics-metric">
                            <p class="analytics-metric-label">"portals used"</p>
                            <p class="analytics-metric-value">
                                (format_number(data.overview.outbound_clicks))
                            </p>
                            <p class="analytics-metric-note">
                                (if data.overview.outbound_clicks == 1 {
                                    "outbound link click"
                                } else {
                                    "outbound link clicks"
                                })
                            </p>
                        </article>
                    </div>
                )

                if let Some(trend) = trend.as_ref() {
                    rail_section(
                        class: "mt-12",
                        stamp: "pulse",
                        <section class="analytics-panel analytics-trend-panel">
                            <header class="analytics-panel-head">
                                <div>
                                    <h2>"Traffic, day by day"</h2>
                                    <p>"Views in oxide; anonymous visitors in steel."</p>
                                </div>
                                <p class="analytics-panel-reading">
                                    "peak " (format_number(trend.maximum)) "/day"
                                </p>
                            </header>
                            <svg
                                class="analytics-trend"
                                viewBox="0 0 720 220"
                                role="img"
                                aria-label=(format!(
                                    "Daily traffic from {} through {}; peak {} page views",
                                    trend.first_date,
                                    trend.last_date,
                                    trend.maximum,
                                ))
                            >
                                <line class="analytics-gridline" x1="0" y1="190" x2="720" y2="190" />
                                <line class="analytics-gridline" x1="0" y1="105" x2="720" y2="105" />
                                <line class="analytics-gridline" x1="0" y1="20" x2="720" y2="20" />
                                <path class="analytics-trend-area" d=(trend.area.as_str()) />
                                <polyline
                                    class="analytics-trend-line"
                                    points=(trend.line.as_str())
                                    fill="none"
                                />
                                for (index, day) in data.days.iter().enumerate() {
                                    if day.views > 0 {
                                        <circle
                                            class="analytics-trend-dot"
                                            cx=(trend_x(index, data.days.len()))
                                            cy=(trend_y(day.views, trend.maximum))
                                            r="3"
                                        >
                                            <title>
                                                (format!(
                                                    "{} · {} · {} · {} attention",
                                                    day.date,
                                                    pluralized(day.views, "view", "views"),
                                                    pluralized(day.visitors, "visitor", "visitors"),
                                                    format_duration(day.engaged_seconds),
                                                ))
                                            </title>
                                        </circle>
                                    }
                                }
                                <text class="analytics-axis-label" x="0" y="214">
                                    (short_date(&trend.first_date))
                                </text>
                                <text
                                    class="analytics-axis-label"
                                    x="720"
                                    y="214"
                                    text-anchor="end"
                                >(short_date(&trend.last_date))</text>
                            </svg>
                            <div class="analytics-trend-legend" aria-hidden="true">
                                <span><i class="analytics-key analytics-key-views"></i>"views"</span>
                                <span>
                                    (pluralized(
                                        data.days.iter().map(|day| day.visitors).sum(),
                                        "daily visitor",
                                        "daily visitors",
                                    ))
                                </span>
                            </div>
                        </section>
                    )
                }

                rail_section(
                    class: "mt-12",
                    stamp: "routes",
                    <div class="analytics-split">
                        <section class="analytics-panel">
                            <header class="analytics-panel-head">
                                <div>
                                    <h2>"Most-read pages"</h2>
                                    <p>"Attention follows the visit across the site."</p>
                                </div>
                            </header>
                            if data.pages.is_empty() {
                                empty_reading(label: "No page views in this window yet.")
                            } else {
                                <ol class="analytics-bars">
                                    for page in data.pages.iter() {
                                        <li>
                                            <div class="analytics-bar-label">
                                                <a href=(page.path.as_str())>(page.path.as_str())</a>
                                                <span>(format_number(page.views))</span>
                                            </div>
                                            <div class="analytics-bar-track">
                                                <i style=(bar_width(
                                                    page.views,
                                                    data.pages.first().map_or(1, |first| first.views),
                                                ))></i>
                                            </div>
                                            <p class="analytics-bar-note">
                                                (pluralized(page.visitors, "visitor", "visitors")) " · "
                                                (format_duration(page.engaged_seconds)) " attention · "
                                                (format!("{}% depth", page.scroll_percent))
                                            </p>
                                        </li>
                                    }
                                </ol>
                            }
                        </section>

                        <section class="analytics-panel">
                            <header class="analytics-panel-head">
                                <div>
                                    <h2>"How people arrive"</h2>
                                    <p>"Acquisition referrers, stripped to their host."</p>
                                </div>
                            </header>
                            if data.channels.is_empty() {
                                empty_reading(label: "No arrivals in this window yet.")
                            } else {
                                <ol class="analytics-bars analytics-bars-compact">
                                    for channel in data.channels.iter() {
                                        <li>
                                            <div class="analytics-bar-label">
                                                <span>(channel_label(&channel.label))</span>
                                                <span>(format_number(channel.count))</span>
                                            </div>
                                            <div class="analytics-bar-track">
                                                <i
                                                    class="analytics-bar-steel"
                                                    style=(bar_width(
                                                        channel.count,
                                                        data.channels.first().map_or(1, |first| first.count),
                                                    ))
                                                ></i>
                                            </div>
                                        </li>
                                    }
                                </ol>
                                if !data.referrers.is_empty() {
                                    <div class="analytics-domain-list">
                                        <p>"referring hosts · ≥3 visitors"</p>
                                        for referrer in data.referrers.iter() {
                                            <span>
                                                (referrer.label.as_str())
                                                <b>(format_number(referrer.views))</b>
                                            </span>
                                        }
                                    </div>
                                }
                            }
                        </section>
                    </div>
                )

                rail_section(
                    class: "mt-12",
                    stamp: "signals",
                    <section class="analytics-panel analytics-map-panel">
                        <header class="analytics-panel-head">
                            <div>
                                <h2>"Signals around the world"</h2>
                                <p>
                                    "Country-level only. A place appears after three anonymous visitors."
                                </p>
                            </div>
                            <p class="analytics-panel-reading">
                                (pluralized(
                                    data.countries.len() as i64,
                                    "place",
                                    "places",
                                ))
                            </p>
                        </header>
                        <div class="analytics-atlas-wrap">
                            <svg
                                class="analytics-atlas"
                                viewBox="0 0 800 400"
                                role="img"
                                aria-label="Approximate world map of public visitor cohorts"
                            >
                                <g class="analytics-graticule">
                                    for x in [100, 200, 300, 400, 500, 600, 700] {
                                        <line x1=(x) y1="18" x2=(x) y2="382" />
                                    }
                                    for y in [80, 140, 200, 260, 320] {
                                        <line x1="18" y1=(y) x2="782" y2=(y) />
                                    }
                                </g>
                                <g class="analytics-land">
                                    <path d="M35 94 L76 53 160 44 220 69 244 105 208 131 176 133 150 169 107 158 78 130 44 125Z" />
                                    <path d="M200 166 L242 180 265 219 248 270 218 326 191 291 185 235Z" />
                                    <path d="M306 87 L352 67 413 73 450 57 523 65 590 54 674 73 746 111 730 148 668 158 625 140 574 157 531 142 490 163 447 145 407 126 365 132 331 117Z" />
                                    <path d="M350 139 L412 143 449 180 437 236 399 299 365 259 341 204Z" />
                                    <path d="M653 249 L700 227 750 248 759 286 724 315 676 303 645 276Z" />
                                    <path d="M759 326 L777 319 786 338 773 351Z" />
                                    <path d="M267 53 L302 28 333 38 322 70 288 76Z" />
                                </g>
                                for marker in markers.iter() {
                                    <g class="analytics-map-marker">
                                        <circle
                                            cx=(format!("{:.2}", marker.x))
                                            cy=(format!("{:.2}", marker.y))
                                            r=(format!("{:.2}", marker.radius + 3.0))
                                            class="analytics-map-halo"
                                        ></circle>
                                        <circle
                                            cx=(format!("{:.2}", marker.x))
                                            cy=(format!("{:.2}", marker.y))
                                            r=(format!("{:.2}", marker.radius))
                                        >
                                            <title>
                                                (format!(
                                                    "{} ({}) · {} · {}",
                                                    marker.name,
                                                    marker.code,
                                                    pluralized(marker.views, "view", "views"),
                                                    pluralized(
                                                        marker.visitors,
                                                        "visitor",
                                                        "visitors",
                                                    ),
                                                ))
                                            </title>
                                        </circle>
                                    </g>
                                }
                            </svg>
                            if markers.is_empty() {
                                <p class="analytics-map-empty">
                                    "The atlas lights up when a country reaches the three-visitor display line."
                                </p>
                            }
                        </div>
                        if !data.countries.is_empty() {
                            <div class="analytics-country-key">
                                for country in data.countries.iter() {
                                    <span>
                                        <b>(country_name(&country.label))</b>
                                        (pluralized(country.views, "view", "views"))
                                    </span>
                                }
                            </div>
                        }
                    </section>
                )

                rail_section(
                    class: "mt-12",
                    stamp: "rhythm",
                    <div class="analytics-split analytics-split-rhythm">
                        <section class="analytics-panel">
                            <header class="analytics-panel-head">
                                <div>
                                    <h2>"When the site gets read"</h2>
                                    <p>"Each visitor's own local clock."</p>
                                </div>
                            </header>
                            <div class="analytics-heat-wrap">
                                <div
                                    class="analytics-heat"
                                    role="img"
                                    aria-label="Page views by local weekday and hour"
                                >
                                    for (weekday, label) in
                                        ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
                                            .iter()
                                            .enumerate()
                                    {
                                        <span class="analytics-heat-day">(label)</span>
                                        for hour in 0..24 {
                                            <i
                                                style=(heat_style(
                                                    data.hourly[weekday][hour],
                                                    heat_max,
                                                ))
                                                title=(format!(
                                                    "{} {:02}:00 · {}",
                                                    label,
                                                    hour,
                                                    pluralized(
                                                        data.hourly[weekday][hour],
                                                        "view",
                                                        "views",
                                                    ),
                                                ))
                                            ></i>
                                        }
                                    }
                                </div>
                                <div class="analytics-heat-hours" aria-hidden="true">
                                    <span>"midnight"</span>
                                    <span>"6am"</span>
                                    <span>"noon"</span>
                                    <span>"6pm"</span>
                                    <span>"midnight"</span>
                                </div>
                            </div>
                        </section>

                        <section class="analytics-panel">
                            <header class="analytics-panel-head">
                                <div>
                                    <h2>"The reading room"</h2>
                                    <p>"Coarse technology groups; rare combinations stay hidden."</p>
                                </div>
                            </header>
                            for (dimension, heading) in [
                                ("device", "device"),
                                ("browser", "browser"),
                                ("os", "operating system"),
                            ] {
                                <div class="analytics-tech">
                                    <p>(heading)</p>
                                    let dimension_max = data
                                        .technology
                                        .iter()
                                        .filter(|item| item.dimension == dimension)
                                        .map(|item| item.views)
                                        .max()
                                        .unwrap_or(1);
                                    for item in data
                                        .technology
                                        .iter()
                                        .filter(|item| item.dimension == dimension)
                                    {
                                        <div title=(pluralized(
                                            item.visitors,
                                            "distinct visitor",
                                            "distinct visitors",
                                        ))>
                                            <span>(item.label.as_str())</span>
                                            <i style=(bar_width(item.views, dimension_max))></i>
                                            <b>(format_number(item.views))</b>
                                        </div>
                                    }
                                </div>
                            }
                        </section>
                    </div>
                )

                rail_section(
                    class: "mt-12",
                    stamp: "quality",
                    <section class="analytics-panel">
                        <header class="analytics-panel-head">
                            <div>
                                <h2>"Reading quality"</h2>
                                <p>
                                    "Engagement and browser performance, measured without fingerprinting."
                                </p>
                            </div>
                            <p class="analytics-panel-reading">
                                (pluralized(
                                    data.performance.samples,
                                    "sample",
                                    "samples",
                                ))
                            </p>
                        </header>
                        <div class="analytics-quality">
                            <article>
                                <p>"active attention"</p>
                                <strong>(format_duration(data.performance.attention_seconds))</strong>
                                <span>"average while visible"</span>
                            </article>
                            <article>
                                <p>"reading depth"</p>
                                <strong>(format!("{}%", data.performance.scroll_percent))</strong>
                                <span>
                                    (format!("{}% reached the end", data.performance.finish_percent))
                                </span>
                            </article>
                            <article>
                                <p>"largest paint"</p>
                                <strong>(format_milliseconds(data.performance.lcp_milliseconds))</strong>
                                <span>"largest contentful paint"</span>
                            </article>
                            <article>
                                <p>"layout shift"</p>
                                <strong>(format_cls(data.performance.cls_thousandths))</strong>
                                <span>"cumulative layout shift"</span>
                            </article>
                            <article>
                                <p>"navigation"</p>
                                <strong>
                                    (format_milliseconds(data.performance.navigation_milliseconds))
                                </strong>
                                <span>"browser navigation timing"</span>
                            </article>
                            <article>
                                <p>"single-page sessions"</p>
                                <strong>(format!("{}%", data.overview.single_page_percent))</strong>
                                <span>"a descriptive count, not a judgment"</span>
                            </article>
                        </div>
                    </section>
                )

                if !data.journeys.is_empty()
                    || !data.outbound.is_empty()
                    || !data.campaigns.is_empty()
                {
                    rail_section(
                        class: "mt-12",
                        stamp: "motion",
                        <div class="analytics-split">
                            <section class="analytics-panel">
                                <header class="analytics-panel-head">
                                    <div>
                                        <h2>"Paths through the workshop"</h2>
                                        <p>"Repeated internal hops, never individual trails."</p>
                                    </div>
                                </header>
                                if data.journeys.is_empty() {
                                    empty_reading(label: "No journey has crossed the display threshold.")
                                } else {
                                    <ol class="analytics-journeys">
                                        for journey in data.journeys.iter() {
                                            <li>
                                                <span>(journey.from.as_str())</span>
                                                <i aria-hidden="true">"→"</i>
                                                <span>(journey.to.as_str())</span>
                                                <b title=(pluralized(
                                                    journey.visitors,
                                                    "distinct visitor",
                                                    "distinct visitors",
                                                ))>(format_number(journey.trips))</b>
                                            </li>
                                        }
                                    </ol>
                                }
                            </section>
                            <section class="analytics-panel">
                                <header class="analytics-panel-head">
                                    <div>
                                        <h2>"Where readers go next"</h2>
                                        <p>"Outbound domains and attributed campaigns."</p>
                                    </div>
                                </header>
                                if data.outbound.is_empty() && data.campaigns.is_empty() {
                                    empty_reading(label: "No destination has crossed the display threshold.")
                                } else {
                                    <div class="analytics-domain-list analytics-domain-list-roomy">
                                        for destination in data.outbound.iter() {
                                            <span>
                                                (destination.label.as_str())
                                                <b>
                                                    (pluralized(
                                                        destination.views,
                                                        "exit",
                                                        "exits",
                                                    ))
                                                </b>
                                            </span>
                                        }
                                        for campaign in data.campaigns.iter() {
                                            <span>
                                                (format!("{} / {}", campaign.source, campaign.campaign))
                                                <b title=(pluralized(
                                                    campaign.visitors,
                                                    "distinct visitor",
                                                    "distinct visitors",
                                                ))>
                                                    (pluralized(
                                                        campaign.views,
                                                        "arrival",
                                                        "arrivals",
                                                    ))
                                                </b>
                                            </span>
                                        }
                                    </div>
                                }
                            </section>
                        </div>
                    )
                }
            } else {
                rail_section(
                    class: "mt-10",
                    stamp: "standby",
                    <section class="analytics-panel analytics-standby">
                        <p class="analytics-standby-light" aria-hidden="true"></p>
                        <div>
                            <h2>"The instruments are warming up."</h2>
                            <p>
                                "There is no readable database snapshot right now. The rest of the \
                                 site remains available, and this panel will recover on its next refresh."
                            </p>
                        </div>
                    </section>
                )
            }

            rail_section(
                class: "mt-14",
                stamp: "private",
                <section class="analytics-private" id="private-ledger">
                    <div class="analytics-private-copy">
                        <p class="analytics-private-kicker">"sign the private ledger"</p>
                        <h2>"A number can introduce itself."</h2>
                        <p>
                            "If you know me — or would like to — leave a name and a note. It is \
                             stored separately from these public aggregates. Only Ben can read it; \
                             this page has no route that can."
                        </p>
                    </div>
                    <form
                        class="analytics-identify"
                        method="post"
                        action="/api/analytics/identify"
                        autocomplete="on"
                    >
                        <input
                            id="analytics-private-bootstrap"
                            name="bootstrap_id"
                            type="hidden"
                            value=""
                        >
                        <label for="analytics-name">
                            <span>"what should I call you?"</span>
                            <input
                                id="analytics-name"
                                name="name"
                                type="text"
                                maxlength="80"
                                autocomplete="name"
                                required=""
                                placeholder="Your name"
                            >
                        </label>
                        <label for="analytics-note">
                            <span>"optional note"</span>
                            <textarea
                                id="analytics-note"
                                name="note"
                                maxlength="400"
                                rows="3"
                                placeholder="Where we met, what brought you here, or simply hello."
                            ></textarea>
                        </label>
                        <label class="analytics-honeypot" aria-hidden="true">
                            <span>"Company"</span>
                            <input
                                name="website"
                                type="text"
                                tabindex="-1"
                                autocomplete="off"
                            >
                        </label>
                        <button type="submit">"Sign privately →"</button>
                    </form>
                    <div class="analytics-thanks" id="thanks" role="status">
                        <p>"Entry received."</p>
                        <span>"Thank you for making the graph a little more human."</span>
                    </div>
                </section>
            )

            rail_section(
                class: "mt-12",
                stamp: "method",
                <section class="analytics-method">
                    <h2>"Deliberately boring surveillance"</h2>
                    <div>
                        <p>
                            "First-party only. No pixels, ad IDs, raw IP addresses, precise \
                             coordinates, fingerprint, or third-party requests."
                        </p>
                        <p>
                            "External referrers stop at the hostname; internal referrers stop at \
                             the path. Query strings are discarded except explicit UTM labels."
                        </p>
                        <p>
                            "Global Privacy Control and Do Not Track are honored. Geography, \
                             technology, referrers, journeys, exits, and campaigns need at least \
                             three anonymous visitors before they appear."
                        </p>
                    </div>
                </section>
            )
        )
    }
}

#[route(POST "/api/analytics/events")]
async fn record_event(cx: &Cx, body: Body) -> Result<Response> {
    if tracking_opted_out(headers(cx)) {
        return Ok(no_content_response());
    }
    if !is_same_origin(headers(cx)) {
        return Ok(json_response(StatusCode::FORBIDDEN, "forbidden"));
    }
    if media_type(cx) != Some("application/json") {
        return Ok(json_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        ));
    }
    if let Some(response) = content_length_response(cx, EVENT_BODY_LIMIT) {
        return Ok(response);
    }
    let bytes = match to_bytes(body, EVENT_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(json_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body is too large",
            ));
        }
    };
    let payload: EventPayload = match serde_json::from_slice(&bytes) {
        Ok(payload) => payload,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, "bad event")),
    };
    let now = epoch_seconds();
    let event = match payload.validate(headers(cx), now) {
        Ok(event) => event,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, "bad event")),
    };
    let bootstrap_id = event.bootstrap_id.clone();

    let existing_hash = session::token_hash(cx).await?;
    let guard_key = existing_hash
        .as_ref()
        .map(token_hash_hex)
        .unwrap_or_else(|| bootstrap_id.clone());
    if !app_context::<AnalyticsGuard>(cx).allow(headers(cx), &guard_key, WriteKind::Event) {
        return Ok(json_response(StatusCode::TOO_MANY_REQUESTS, "slow down"));
    }
    let token_hash = match existing_hash {
        Some(hash) => token_hash_hex(&hash),
        None => token_hash_hex(&session::start(cx).await?.token_hash),
    };

    let result = async {
        let db = app_context::<Data>(cx).db().await?;
        let visitor_hash = db::resolve_visitor(&db, &token_hash, Some(&bootstrap_id), now).await?;
        db::insert_event(&db, &visitor_hash, event, now).await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    match result {
        Ok(()) => Ok(no_content_response()),
        Err(error) => {
            log_failure("/api/analytics/events", error);
            Ok(json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily unavailable",
            ))
        }
    }
}

#[route(POST "/api/analytics/identify")]
async fn identify(cx: &Cx, body: Body) -> Result<Response> {
    if !is_same_origin(headers(cx)) || !current_page_matches(headers(cx), "/analytics") {
        return Ok(plain_response(StatusCode::FORBIDDEN, "forbidden"));
    }
    if media_type(cx) != Some("application/x-www-form-urlencoded") {
        return Ok(plain_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported form",
        ));
    }
    if let Some(response) = content_length_response(cx, IDENTITY_BODY_LIMIT) {
        return Ok(response);
    }
    let bytes = match to_bytes(body, IDENTITY_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(plain_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            ));
        }
    };
    let input = match parse_identity(&bytes) {
        Ok(input) => input,
        Err(_) => return Ok(plain_response(StatusCode::BAD_REQUEST, "bad form")),
    };
    // Quiet success keeps the honeypot useful without persisting its contents.
    if input.honeypot {
        return Ok(identity_redirect());
    }

    let existing_hash = session::token_hash(cx).await?;
    let guard_key = existing_hash
        .as_ref()
        .map(token_hash_hex)
        .or_else(|| input.bootstrap_id.clone())
        .unwrap_or_else(|| "new-private-ledger-entry".to_string());
    if !app_context::<AnalyticsGuard>(cx).allow(headers(cx), &guard_key, WriteKind::Identity) {
        return Ok(plain_response(StatusCode::TOO_MANY_REQUESTS, "slow down"));
    }
    let token_hash = match existing_hash {
        Some(hash) => token_hash_hex(&hash),
        None => token_hash_hex(&session::start(cx).await?.token_hash),
    };
    let result = async {
        let db = app_context::<Data>(cx).db().await?;
        let now = epoch_seconds();
        let visitor_hash =
            db::resolve_visitor(&db, &token_hash, input.bootstrap_id.as_deref(), now).await?;
        db::upsert_identity(&db, &visitor_hash, input.display_name, input.note, now).await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    match result {
        Ok(()) => Ok(identity_redirect()),
        Err(error) => {
            // Never include the submitted body or identity in the log.
            log_failure("/api/analytics/identify", error);
            Ok(plain_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily unavailable",
            ))
        }
    }
}

#[route(GET "/api/analytics/{*rest}")]
async fn unknown_analytics_get() -> Result<Response> {
    Ok(json_response(StatusCode::NOT_FOUND, "not found"))
}

#[route(POST "/api/analytics/{*rest}")]
async fn unknown_analytics_post() -> Result<Response> {
    Ok(json_response(StatusCode::NOT_FOUND, "not found"))
}

fn parse_identity(body: &[u8]) -> Result<IdentityInput, &'static str> {
    std::str::from_utf8(body).map_err(|_| "bad encoding")?;
    let mut name = None;
    let mut note = None;
    let mut website = None;
    let mut bootstrap_id = None;
    let mut legacy_session_id = None;
    for (key, value) in form_urlencoded::parse(body) {
        let slot = match key.as_ref() {
            "name" => &mut name,
            "note" => &mut note,
            "website" => &mut website,
            "bootstrap_id" => &mut bootstrap_id,
            // A cached copy of the original, unreleased form used this name.
            "session_id" => &mut legacy_session_id,
            _ => return Err("unknown field"),
        };
        if slot.replace(value.into_owned()).is_some() {
            return Err("duplicate field");
        }
    }
    let display_name = clean_identity_text(name.as_deref(), 1, 80).ok_or("bad name")?;
    let note = match note.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) => Some(clean_identity_text(Some(value), 1, 400).ok_or("bad note")?),
    };
    let bootstrap_id = match (bootstrap_id.as_deref(), legacy_session_id.as_deref()) {
        (Some(_), Some(_)) => return Err("duplicate bootstrap"),
        (Some(""), None) | (None, Some("")) | (None, None) => None,
        (Some(value), None) | (None, Some(value)) if is_canonical_v4_uuid(value) => {
            Some(value.to_ascii_lowercase())
        }
        _ => return Err("bad bootstrap"),
    };
    Ok(IdentityInput {
        display_name,
        note,
        bootstrap_id,
        honeypot: website.is_some_and(|value| !value.trim().is_empty()),
    })
}

fn clean_identity_text(value: Option<&str>, min: usize, max: usize) -> Option<String> {
    let value = value?;
    if value.chars().any(char::is_control) {
        return None;
    }
    let value = value.trim();
    let len = value.chars().count();
    (min..=max).contains(&len).then(|| value.to_string())
}

fn media_type(cx: &Cx) -> Option<&str> {
    headers(cx)
        .get(header::CONTENT_TYPE)?
        .to_str()
        .ok()?
        .split(';')
        .next()
        .map(str::trim)
}

fn content_length_response(cx: &Cx, limit: usize) -> Option<Response> {
    let value = headers(cx).get(header::CONTENT_LENGTH)?;
    let declared = match value.to_str() {
        Ok(value) => value,
        Err(_) => {
            return Some(plain_response(
                StatusCode::BAD_REQUEST,
                "bad Content-Length",
            ));
        }
    };
    match declared.trim().parse::<usize>() {
        Ok(length) if length <= limit => None,
        Ok(_) => Some(plain_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body is too large",
        )),
        Err(_) => Some(plain_response(
            StatusCode::BAD_REQUEST,
            "bad Content-Length",
        )),
    }
}

fn token_hash_hex(hash: &session::TokenHash) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn json_response(status: StatusCode, message: &str) -> Response {
    let body = serde_json::json!({ "error": message }).to_string();
    response(status, JSON_CONTENT_TYPE, body, None)
}

fn plain_response(status: StatusCode, message: &'static str) -> Response {
    response(
        status,
        "text/plain; charset=utf-8",
        message.to_string(),
        None,
    )
}

fn no_content_response() -> Response {
    response(
        StatusCode::NO_CONTENT,
        "text/plain; charset=utf-8",
        String::new(),
        None,
    )
}

fn identity_redirect() -> Response {
    response(
        StatusCode::SEE_OTHER,
        "text/plain; charset=utf-8",
        "see other".to_string(),
        Some("/analytics#thanks"),
    )
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: String,
    location: Option<&'static str>,
) -> Response {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
        .header(header::VARY, "Origin");
    if let Some(location) = location {
        builder = builder.header(header::LOCATION, location);
    }
    builder
        .body(Body::from(body))
        .expect("analytics response uses static headers")
}

fn log_failure(path: &str, error: impl std::fmt::Display) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "analytics operation failed",
            "path": path,
            "error": error.to_string(),
        })
    );
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn trend_geometry(days: &[Day]) -> Trend {
    let maximum = days.iter().map(|day| day.views).max().unwrap_or(0).max(1);
    let mut points = Vec::with_capacity(days.len());
    for (index, day) in days.iter().enumerate() {
        points.push(format!(
            "{:.2},{:.2}",
            trend_x(index, days.len()),
            trend_y(day.views, maximum),
        ));
    }
    let line = points.join(" ");
    let area = if points.is_empty() {
        String::new()
    } else {
        format!(
            "M 0 190 L {} L 720 190 Z",
            points
                .iter()
                .map(|point| point.replace(',', " "))
                .collect::<Vec<_>>()
                .join(" L ")
        )
    };
    Trend {
        line,
        area,
        maximum,
        first_date: days
            .first()
            .map_or_else(String::new, |day| day.date.clone()),
        last_date: days.last().map_or_else(String::new, |day| day.date.clone()),
    }
}

fn trend_x(index: usize, count: usize) -> f64 {
    if count <= 1 {
        360.0
    } else {
        index as f64 / (count - 1) as f64 * 720.0
    }
}

fn trend_y(value: i64, maximum: i64) -> f64 {
    190.0 - value as f64 / maximum.max(1) as f64 * 170.0
}

fn map_markers(countries: &[Cohort]) -> Vec<MapMarker> {
    let maximum = countries
        .iter()
        .map(|country| country.views)
        .max()
        .unwrap_or(1)
        .max(1);
    countries
        .iter()
        .filter_map(|country| {
            let place = map::place(&country.label)?;
            Some(MapMarker {
                code: country.label.clone(),
                name: place.name,
                views: country.views,
                visitors: country.visitors,
                x: place.x,
                y: place.y,
                radius: 4.0 + (country.views as f64 / maximum as f64).sqrt() * 10.0,
            })
        })
        .collect()
}

fn country_name(code: &str) -> &str {
    map::place(code).map_or(code, |place| place.name)
}

fn format_number(value: i64) -> String {
    let negative = value.is_negative();
    let digits = value.unsigned_abs().to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3 + usize::from(negative));
    if negative {
        output.push('-');
    }
    for (index, byte) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(char::from(byte));
    }
    output
}

fn pluralized(value: i64, singular: &str, plural: &str) -> String {
    format!(
        "{} {}",
        format_number(value),
        if value == 1 { singular } else { plural }
    )
}

fn format_duration(seconds: i64) -> String {
    match seconds.max(0) {
        0..=59 => format!("{}s", seconds.max(0)),
        60..=3_599 => format!("{}m", seconds / 60),
        value => {
            let hours = value / 3_600;
            let minutes = value % 3_600 / 60;
            if minutes == 0 {
                format!("{hours}h")
            } else {
                format!("{hours}h {minutes:02}m")
            }
        }
    }
}

fn format_milliseconds(milliseconds: i64) -> String {
    if milliseconds <= 0 {
        "—".to_string()
    } else if milliseconds < 1_000 {
        format!("{milliseconds}ms")
    } else {
        format!("{:.2}s", milliseconds as f64 / 1_000.0)
    }
}

fn format_cls(thousandths: i64) -> String {
    if thousandths <= 0 {
        "0".to_string()
    } else {
        format!("{:.3}", thousandths as f64 / 1_000.0)
    }
}

fn short_date(date: &str) -> String {
    let mut parts = date.split('-');
    let _year = parts.next();
    match (parts.next(), parts.next()) {
        (Some(month), Some(day)) => format!("{month}/{day}"),
        _ => date.to_string(),
    }
}

fn bar_width(value: i64, maximum: i64) -> String {
    let percent = value.max(0) as f64 / maximum.max(1) as f64 * 100.0;
    format!("width: {percent:.2}%")
}

fn heat_style(value: i64, maximum: i64) -> String {
    let intensity = if value == 0 {
        0.04
    } else {
        0.18 + value as f64 / maximum.max(1) as f64 * 0.82
    };
    format!("--heat: {intensity:.3}")
}

fn channel_label(channel: &str) -> &str {
    match channel {
        "direct" => "Direct / unknown",
        "internal" => "Around this site",
        "search" => "Search",
        "social" => "Social",
        "ai" => "AI assistants",
        "referral" => "Other referrals",
        other => other,
    }
}

#[topcoat::view::component]
async fn empty_reading(label: &str) -> Result {
    view! {
        <p class="analytics-empty">(label)</p>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_parser_is_strict_bounded_and_has_a_honeypot() {
        let bootstrap = uuid::Uuid::new_v4();
        let body = format!("name=Ada+Lovelace&note=hello&website=&bootstrap_id={bootstrap}");
        let parsed = parse_identity(body.as_bytes()).unwrap();
        assert_eq!(parsed.display_name, "Ada Lovelace");
        assert_eq!(parsed.note.as_deref(), Some("hello"));
        assert_eq!(parsed.bootstrap_id, Some(bootstrap.to_string()));
        assert!(!parsed.honeypot);

        assert!(parse_identity(b"name=Ada&name=Grace").is_err());
        assert!(parse_identity(b"name=Ada&bootstrap_id=not-a-uuid").is_err());
        assert!(
            parse_identity(
                format!("name=Ada&bootstrap_id={bootstrap}&session_id={bootstrap}").as_bytes()
            )
            .is_err()
        );
        assert!(
            parse_identity(format!("name=Ada&session_id={bootstrap}").as_bytes())
                .unwrap()
                .bootstrap_id
                .is_some()
        );
        assert!(parse_identity(b"name=Ada&admin=true").is_err());
        assert!(parse_identity(b"name=%0AAda").is_err());

        let bot = parse_identity(b"name=Ada&website=spam.example").unwrap();
        assert!(bot.honeypot);
    }

    #[test]
    fn number_and_duration_formatting_are_stable() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(1_234_567), "1,234,567");
        assert_eq!(format_number(-12_345), "-12,345");
        assert_eq!(pluralized(1, "view", "views"), "1 view");
        assert_eq!(pluralized(1_234, "view", "views"), "1,234 views");
        assert_eq!(format_duration(59), "59s");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3_661), "1h 01m");
    }

    #[test]
    fn trend_geometry_handles_empty_and_single_day_series() {
        let empty = trend_geometry(&[]);
        assert!(empty.line.is_empty());
        assert!(empty.area.is_empty());

        let one = trend_geometry(&[Day {
            date: "2026-07-23".to_string(),
            views: 4,
            visitors: 2,
            engaged_seconds: 10,
        }]);
        assert!(one.line.starts_with("360.00,"));
        assert_eq!(one.maximum, 4);
    }

    #[test]
    fn responses_never_cache_or_enable_cors() {
        let response = json_response(StatusCode::BAD_REQUEST, "bad event");
        assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }
}
