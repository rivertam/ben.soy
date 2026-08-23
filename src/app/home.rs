//! `~`: the front door is the logbook itself. Desktop renders the timeline
//! straight — the tmux windows are the site map — while phones get the pane
//! deck: log ⇄ felix ⇄ fitness ⇄ résumé as side-by-side panes (real swipe
//! via CSS scroll-snap; `browser/panes.js` keeps the tab bar and the hash
//! honest), plus a netrw-style `more` pane listing everything else. Granted
//! hidden pages join that listing as dotfiles for their viewers only (the
//! response layer keeps those renders out of the CDN). Desktop reaches the
//! listing too: the hero's "multitudes" targets `#more`, which unfolds
//! below the timeline. Old `/interests` bookmarks land here.

use benjisponge::data::Data;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{HeaderValue, error::redirect_permanent, header, page, route},
    view::view,
};

use super::login::viewer;
use crate::{
    components::shell,
    content::{access, interests::INTERESTS},
};

const PANES_JS: Asset = asset!("../components/browser/panes.js");

/// Interests promoted to their own deck pane (and pane-bar tab); the `more`
/// listing carries the rest. Tests below hold this in line with the interest
/// registry and the shell's `PANE_TABS`.
const PANE_INTERESTS: [&str; 2] = ["felix", "fitness"];

/// One `more` listing row: the name as netrw would print it (directories
/// keep their trailing slash, hidden pages wear a leading dot), where it
/// goes, and the registry teaser as the long-listing annotation.
struct Listing {
    name: String,
    href: String,
    blurb: &'static str,
    hidden: bool,
}

#[page("/")]
async fn home(cx: &Cx) -> Result {
    // Allowlisted hidden pages join the `more` listing for their viewers
    // only, the way netrw shows dotfiles to people who ask.
    let current = viewer(cx);
    let can_log = current
        .as_ref()
        .is_some_and(|current| access::is_admin(&current.email));
    let granted: Vec<&access::HiddenPage> = match current.as_ref() {
        Some(current) => access::visible_pages(app_context::<Data>(cx), &current.email).await,
        None => Vec::new(),
    };

    let mut rows: Vec<Listing> = INTERESTS
        .iter()
        .filter(|interest| !PANE_INTERESTS.contains(&interest.slug))
        .map(|interest| Listing {
            name: format!("{}/", interest.slug),
            href: format!("/{}", interest.slug),
            blurb: interest.teaser,
            hidden: false,
        })
        .collect();
    rows.extend(granted.iter().map(|page| Listing {
        name: format!(".{}/", page.stamp),
        href: page.path.to_string(),
        blurb: page.teaser,
        hidden: true,
    }));
    rows.push(Listing {
        name: "feed.xml".to_string(),
        href: "/feed.xml".to_string(),
        blurb: "the log, syndicated",
        hidden: false,
    });

    view! {
        // Fresh runs and lifts appear within a minute; CDN honors s-maxage
        // (see docs/railway-deploy.md). The embedded lifting pane rides the
        // same TTL — only the standalone /fitness page stays no-store.
        ((header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=0, s-maxage=60")))
        shell(page: "", active: "~", fitness_pwa: true,
        <div class="pane-deck" data-pane-deck="">
            <section class="pane pane-log" id="log" data-pane="" aria-label="the log">
                crate::app::log::timeline()
            </section>
            <section class="pane pane-felix" id="felix" data-pane="" aria-label="felix">
                crate::app::interests::felix::felix_content(initial_photo: "", standalone: false)
            </section>
            <section class="pane" id="fitness" data-pane="" aria-label="fitness">
                crate::app::interests::lifting::home::fitness_home_content()
            </section>
            <section class="pane" id="resume" data-pane="" aria-label="résumé">
                crate::app::resume::resume_content()
            </section>
            // The `more` pane: a netrw-flavored flat listing of everything
            // without a pane of its own, closed by the session line and the
            // social row (the shell footer is hidden while the deck owns the
            // phone viewport).
            <section class="pane pane-more" id="more" data-pane="" aria-label="everything else">
                <section class="netrw mt-6 font-meta text-[13px] sm:mt-16 sm:text-sm">
                    <h2 class="sr-only">"Everything else"</h2>
                    <p class="text-muted">"~/ everything else"</p>
                    <ul class="mt-5 space-y-4 sm:mt-2 sm:space-y-1">
                        for row in rows.iter() {
                            <li
                                class="netrw-row relative"
                                data-rail-item=""
                                data-rail-href=(row.href.as_str())
                            >
                                <a
                                    class=(if row.hidden {
                                        "group flex flex-col gap-y-0.5 no-underline opacity-75 sm:flex-row sm:items-baseline sm:gap-x-5"
                                    } else {
                                        "group flex flex-col gap-y-0.5 no-underline sm:flex-row sm:items-baseline sm:gap-x-5"
                                    })
                                    href=(row.href.as_str())
                                >
                                    <span class=(if row.name.ends_with('/') {
                                        "text-oxide group-hover:underline sm:w-36 sm:shrink-0"
                                    } else {
                                        "text-ink group-hover:underline sm:w-36 sm:shrink-0"
                                    })>(row.name.as_str())</span>
                                    <span class="min-w-0 text-muted sm:truncate">(row.blurb)</span>
                                </a>
                            </li>
                        }
                    </ul>
                    <div class="mt-10 border-t border-hairline pt-4 text-muted">
                        <p>
                            "\" Session:  "
                            if current.is_some() {
                                <a
                                    class="netrw-login"
                                    href="/login?next=%2F"
                                    data-modal-open="account-dialog"
                                    aria-label="signed-in account"
                                >"signed in"</a>
                                " · "
                                <a
                                    class="netrw-logout cursor-pointer"
                                    href="/login?next=%2F"
                                    data-modal-open="account-dialog"
                                >"logout"</a>
                            } else {
                                <a
                                    class="netrw-login"
                                    href="/login?next=%2F"
                                    data-modal-open="account-dialog"
                                    aria-label="log in"
                                >"login"</a>
                            }
                        </p>
                        <p class="mt-3 flex flex-wrap gap-x-5 gap-y-1">
                            <a href="https://www.linkedin.com/in/benmberman" class="quiet-link">"LinkedIn"</a>
                            <a href="https://github.com/rivertam" class="quiet-link">"GitHub"</a>
                            <a href="https://www.reddit.com/user/BenjiSponge" class="quiet-link">"Reddit"</a>
                        </p>
                    </div>
                </section>
            </section>
            <script type="module" src=(PANES_JS)></script>
        </div>
        // Both the log and fitness headers point at one dialog set. Keeping
        // it outside the pane deck also keeps the native top-layer surfaces
        // out of the desktop-hidden fitness pane.
        if can_log {
            crate::app::interests::lifting::home::log_dialogs()
        }
        // A visitor can still reach Fitness by swiping the home deck instead
        // of following the canonical tab link. Advertise and register the
        // same narrowly scoped app here so installing from that live pane
        // still launches `/fitness`, never a generic `/` shortcut.
        <script
            type="module"
            src=(crate::app::interests::running::PWA_JS)
        ></script>
        )
    }
}

/// The old interests index, folded into the `more` listing: bookmarks land
/// on `~`. (The per-page `/interests/{slug}` redirects live with their
/// pages.)
#[route(GET "/interests")]
async fn legacy_interests() -> Result {
    Err(redirect_permanent("/").into())
}

#[cfg(test)]
mod tests {
    use super::PANE_INTERESTS;
    use crate::components::chrome::PANE_TABS;
    use crate::content::interests::INTERESTS;

    const HOME_SRC: &str = include_str!("home.rs");

    /// The shell's pane-bar tabs and the deck's panes are two hand-written
    /// lists; every tab must address a pane id rendered here, and nothing
    /// but the five panes may carry the deck marker.
    #[test]
    fn every_pane_tab_has_a_deck_pane() {
        for tab in PANE_TABS {
            assert!(
                HOME_SRC.contains(&format!("id=\"{tab}\"")),
                "pane tab `{tab}` has no deck pane"
            );
        }
        assert_eq!(
            HOME_SRC.matches("data-pane=\"\"").count(),
            PANE_TABS.len(),
            "deck panes and pane tabs diverged"
        );
    }

    /// Promoted interests must exist in the registry and on the tab bar;
    /// everything else lands in the `more` listing.
    #[test]
    fn promoted_interests_are_registered_and_tabbed() {
        for slug in PANE_INTERESTS {
            assert!(
                INTERESTS.iter().any(|interest| interest.slug == slug),
                "promoted pane `{slug}` is not a registered interest"
            );
            assert!(
                PANE_TABS.contains(&slug),
                "promoted interest `{slug}` is missing from the pane tabs"
            );
        }
    }

    /// The Fitness pane can still be reached by swiping rather than by its
    /// canonical tab link. That document must advertise and register the same
    /// narrowly scoped app or Android will offer a generic `/` shortcut.
    #[test]
    fn home_deck_can_install_the_fitness_app() {
        assert!(HOME_SRC.contains("fitness_pwa: true"));
        assert!(HOME_SRC.contains("running::PWA_JS"));
    }
}
