//! The hidden `/podrick` page.
//!
//! Podrick himself is `podrick.rs`, a separate binary in this folder that runs
//! as its own service (`docs/podrick.md`). This module is only the page that
//! renders the Pants Off calendars — the lift job has no panel here; you read
//! it in the channel it posts to. It follows the hidden page contract in
//! `docs/auth.md`: database-managed grants, no public registry, no analytics,
//! and no-store before every rendered shell.

mod heatmap;
mod seed;
pub(crate) mod status;

use benjisponge::data::Data;
use jiff::Timestamp;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{HeaderValue, error::redirect, header, page, query_params, uri},
    view::view,
};

use crate::{
    components::{back_link, page_head, rail_section, shell},
    content::access::{hidden_page, may_view},
};

use super::super::{login::viewer, not_found::not_found_page};

const PATH: &str = "/podrick";
const LOGIN_REDIRECT: &str = "/login?next=%2Fpodrick";

/// The anthem. An AI-written single that plays on arrival (as far as
/// autoplay policy allows — pants-off.js holds the graceful fallback),
/// under every theme; its data-page-band attribute tells the theme tune
/// engine this page brought its own band. The asset URL is unauthenticated
/// like every hashed asset, which matches the hidden-page contract: the
/// PAGE is gated, the repo and its bytes were never secret.
const ANTHEM_MP3: Asset = asset!("./pants-off-oclock.mp3");
const ANTHEM_JS: Asset = asset!("./pants-off.js");

#[query_params(error = redirect("?"))]
struct PodrickQuery {
    year: Option<i16>,
}

fn selected_year(requested: Option<i16>, earliest: i16, current: i16) -> i16 {
    requested.unwrap_or(current).clamp(earliest, current)
}

fn canonical_year_query(year: i16, current: i16) -> Option<String> {
    (year != current).then(|| format!("year={year}"))
}

#[page("/podrick")]
async fn podrick(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return Err(redirect(LOGIN_REDIRECT).into());
    };
    if !may_view(app_context::<Data>(cx), &current.email, PATH).await {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
            not_found_page(requested: PATH)
        };
    }
    let meta = hidden_page(PATH).expect("/podrick is a registered hidden page");
    let query = query_params::<PodrickQuery>(cx)?;
    let now = Timestamp::now().as_second();
    let pants = status::load(app_context::<Data>(cx)).await;
    let (earliest_year, current_year) =
        heatmap::pants_year_bounds(&pants, now).unwrap_or((1970, 1970));
    let selected_year = selected_year(query.year, earliest_year, current_year);
    let canonical_query = canonical_year_query(selected_year, current_year);
    if uri(cx).query() != canonical_query.as_deref() {
        return Err(redirect(&heatmap::year_path(selected_year, current_year)).into());
    }

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            title: meta.title,
            active: "",
            runtime: false,
            analytics: false,
            page_head(stamp: meta.stamp, title: meta.title, lede: meta.teaser)

            rail_section(
                class: "mt-8",
                stamp: "anthem",
                <div class="flex flex-wrap items-baseline gap-x-4 gap-y-2">
                    // data-music-toggle: the chip is another view of the
                    // site-wide music preference (with the corner pill and
                    // the switcher's ♪ row); theme.js drives playback.
                    <button
                        type="button"
                        id="anthem-toggle"
                        class="chip chip-oxide cursor-pointer"
                        data-music-toggle=""
                        aria-pressed="false"
                    >"▶ pants off o'clock"</button>
                </div>
                // preload=metadata, not none: Chromium's deferred-load path
                // (none + play()) can wedge at readyState 0; metadata keeps
                // the arrival fetch to headers while play() starts from the
                // reliable ready state.
                <audio id="anthem" data-page-band="" preload="metadata" src=(ANTHEM_MP3)></audio>
                <script type="module" src=(ANTHEM_JS)></script>
            )

            rail_section(
                class: "mt-8",
                stamp: "history",
                heatmap::pants_heatmaps(
                    status: pants,
                    now: now,
                    selected_year: selected_year,
                    earliest_year: earliest_year,
                    current_year: current_year
                )
            )

            back_link(href: "/interests", label: "all interests")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_selection_defaults_and_clamps_to_history() {
        assert_eq!(selected_year(None, 2023, 2026), 2026);
        assert_eq!(selected_year(Some(2024), 2023, 2026), 2024);
        assert_eq!(selected_year(Some(1900), 2023, 2026), 2023);
        assert_eq!(selected_year(Some(3000), 2023, 2026), 2026);
    }

    #[test]
    fn current_year_has_the_bare_canonical_url() {
        assert_eq!(canonical_year_query(2026, 2026), None);
        assert_eq!(heatmap::year_path(2026, 2026), "/podrick");
        assert_eq!(
            canonical_year_query(2025, 2026).as_deref(),
            Some("year=2025")
        );
        assert_eq!(heatmap::year_path(2025, 2026), "/podrick?year=2025");
    }

    #[test]
    fn hidden_page_login_redirect_remains_query_free() {
        assert_eq!(LOGIN_REDIRECT, "/login?next=%2Fpodrick");
    }
}
