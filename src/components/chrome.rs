//! The document shell: fonts, head, nav, footer. Every page renders through
//! this.

use benjisponge::data::Data;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    font::{Font, fontsource::fontsource_font},
    router::{HeaderValue, header, uri},
    view::{Unescaped, View, component, view},
};

use crate::app::login::{
    POPUP_ERROR_PARAM, auth_return_target, login_configured, popup_notice, viewer,
};
use crate::components::{back_link, modal};
use crate::content::{access, interests::INTERESTS, logbook::LOG, posts::post_for_path, themes};
use crate::util::urlencode;

pub const ZILLA_SLAB: Font = fontsource_font!(ZILLA_SLAB, host: Asset);
pub const FIRA_SANS: Font = fontsource_font!(FIRA_SANS, host: Asset);
pub const FIRA_MONO: Font = fontsource_font!(FIRA_MONO, host: Asset);
const KALAM: Font = fontsource_font!(KALAM, weight: 700, style: Normal, subset: Latin, host: Asset);
/// Clown mode's font. Linked on every page but fetched only when a
/// `[data-theme="clown"]` stack actually uses it (@font-face is lazy), so
/// the joke lands on Linux too — fontconfig maps Comic Sans to Noto Sans,
/// which is nobody's idea of funny.
const COMIC_NEUE: Font = fontsource_font!(COMIC_NEUE, host: Asset);
/// The site stylesheet's ONE declaration — `stylesheet!()` is an `asset!`
/// underneath, and a second invocation anywhere registers a duplicate
/// serving route that panics at router build (which `just check` never
/// runs). diary_sync.rs resolves this const into the /diary-sync.js loader
/// so the service worker's offline SSR can link the same stylesheet.
pub const SITE_CSS: Asset = topcoat::tailwind::stylesheet!();
// Browser behavior is committed as native modules. Topcoat fingerprints each
// source during the ordinary asset-bundle step, so no Node/TypeScript
// toolchain or generated JS needs to enter the Rust-only build. Theme package
// entry points and their optional assets live in content/themes.rs.
const APPEARANCE_JS: Asset = asset!("./browser/appearance.js");
const AUTH_DIALOG_JS: Asset = asset!("./browser/auth-dialog.js");
const MODALS_JS: Asset = asset!("./browser/modals.js");
const NAVIGATION_JS: Asset = asset!("./browser/navigation.js");
const NAVIGATION_HINTS_JS: Asset = asset!("./browser/navigation/hints.js");
const SESSION_JS: Asset = asset!("./browser/session.js");
const SESSION_VIMIUM_JS: Asset = asset!("./browser/session/vimium.js");
const FAVICON_16: Asset = asset!("./favicon/favicon-16.png");
const FAVICON_32: Asset = asset!("./favicon/favicon-32.png");
const APPLE_TOUCH_ICON: Asset = asset!("./favicon/apple-touch-icon.png");

/// One window in the site's session bar: a precomputed `"3 felix"`
/// label, its destination, and whether the request path lives inside it.
struct SessionWindow {
    label: String,
    href: String,
    current: bool,
}

/// The default session's windows: the site's full flat map (~ — the logbook
/// home — the résumé, each interest, any granted hidden pages) — a superset
/// of the header's fixed links — numbered in render order.
/// The current window is the longest matching path prefix — `/fitness/log`
/// lights `fitness` while `/thoughts/anything` stays home at `~`.
fn session_windows(path: &str, hidden_pages: &[&'static access::HiddenPage]) -> Vec<SessionWindow> {
    let mut windows: Vec<(String, String)> = vec![
        ("~".to_string(), "/".to_string()),
        ("resume".to_string(), "/resume".to_string()),
    ];
    windows.extend(
        INTERESTS
            .iter()
            .map(|interest| (interest.slug.to_string(), format!("/{}", interest.slug))),
    );
    windows.extend(
        hidden_pages
            .iter()
            .map(|hidden| (hidden.stamp.to_string(), hidden.path.to_string())),
    );
    let active = windows
        .iter()
        .enumerate()
        .filter(|(_, (_, href))| {
            let href = href.as_str();
            href == "/" || path == href || path.starts_with(&format!("{href}/"))
        })
        .max_by_key(|(_, (_, href))| href.len())
        .map_or(0, |(index, _)| index);
    windows
        .into_iter()
        .enumerate()
        .map(|(index, (name, href))| SessionWindow {
            label: format!("{index} {name}"),
            href,
            current: index == active,
        })
        .collect()
}

/// The phone tab bar's five panes, in deck order. Home renders one deck pane
/// per entry (`app/home.rs` — its tests hold the two lists together); the
/// bar renders on every page so phone visitors always have the map.
pub(crate) const PANE_TABS: [&str; 5] = ["log", "felix", "fitness", "resume", "more"];

/// The pane tab a path lives under: the log owns home and its posts, the two
/// promoted interests and the résumé own themselves, and "more" covers every
/// other interest plus any granted hidden page. Pages outside the map
/// (/diary, /admin, /login…) light nothing.
fn pane_tab(path: &str, hidden_pages: &[&'static access::HiddenPage]) -> Option<&'static str> {
    let within = |root: &str| path == root || path.starts_with(&format!("{root}/"));
    if path == "/" || within("/thoughts") {
        return Some("log");
    }
    for tab in ["felix", "fitness", "resume"] {
        if within(&format!("/{tab}")) {
            return Some(tab);
        }
    }
    let more = INTERESTS
        .iter()
        .any(|interest| within(&format!("/{}", interest.slug)))
        || hidden_pages.iter().any(|page| within(page.path));
    more.then_some("more")
}

/// A pane tab's destination. On `/` the tabs address the deck's panes by
/// bare fragment (no reload, and active filters in the query survive);
/// everywhere else they carry the visitor home first.
fn pane_href(at_home: bool, tab: &str) -> String {
    match (at_home, tab) {
        (true, _) => format!("#{tab}"),
        (false, "log") => "/".to_string(),
        (false, _) => format!("/#{tab}"),
    }
}

/// The full document: every page renders through this, so every page owns its
/// title. Pages invoke it as markup with the page content as trailing children:
/// `view! { shell(title: "…", active: "…", <p>"…"</p>) }`.
///
/// `title` is the bare page title — the shell appends "— Ben Berman" itself;
/// pass `""` for the homepage, whose title is just the name.
///
/// `active` names the nav item the page lives under — `"~"`, `"log"`,
/// `"resume"`, or `""` for none — and gets the oxide underline.
///
/// `hide_nav` removes the header for an immersive, self-contained page.
///
/// `runtime` controls Topcoat's browser runtime. It defaults on for existing
/// pages; fully server-rendered pages can opt out and ship no production JS.
///
/// `pwa` links the /diary app manifest; `fitness_pwa` links the separate
/// /fitness-scoped app whose Android share target accepts fitness links.
/// Keeping these explicit prevents the diary service worker from ever
/// broadening onto public pages.
///
/// `marker_font` loads Kalam for the handwritten caption on the dog-age post.
/// It defaults off so the extra face does not ride along on every page.
///
/// Signed-in viewers get two quiet extras: their allowlisted hidden pages
/// join the session windows (and home's `more` listing renders them as
/// dotfiles), and a barely-there "signed in" line replaces
/// the footer's login link. The `more` listing and default session bar also
/// expose the door as a small terminal action. These personalize the HTML,
/// which is why
/// `response_layer.rs` forces `private, no-store` whenever the viewer cookie
/// rides the request — the header below only governs anonymous renders.
#[component]
pub async fn shell(
    cx: &Cx,
    title: &str,
    active: &str,
    #[default(false)] hide_nav: bool,
    #[default(true)] runtime: bool,
    #[default(false)] pwa: bool,
    #[default(false)] fitness_pwa: bool,
    #[default(false)] marker_font: bool,
    child: View,
) -> Result {
    let title = if title.is_empty() {
        "Ben Berman".to_string()
    } else {
        format!("{title} — Ben Berman")
    };
    let title = title.as_str();
    let nav = |item: &str| {
        if active == item {
            "nav-active"
        } else {
            "quiet-link"
        }
    };
    let nav_hidden = if hide_nav { "true" } else { "false" };
    let signed_in = viewer(cx);
    // One tiny grants query per signed-in render; anonymous renders (the
    // cacheable majority) never touch the database.
    let hidden_pages = match signed_in.as_ref() {
        Some(current) => access::visible_pages(app_context::<Data>(cx), &current.email).await,
        None => Vec::new(),
    };
    // The session bar is the default navigation. It rides along in every
    // render because cached HTML cannot know whether localStorage will apply
    // an alternate finish before paint.
    let request_uri = uri(cx);
    let post = post_for_path(request_uri.path());
    let return_to = auth_return_target(cx);
    let login_href = format!("/login?next={}", urlencode(&return_to));
    let windows = session_windows(request_uri.path(), &hidden_pages);
    let active_pane = pane_tab(request_uri.path(), &hidden_pages);
    let at_home = request_uri.path() == "/";
    let pane_title = windows
        .iter()
        .find(|win| win.current)
        .map(|win| win.label.clone())
        .unwrap_or_default();
    let theme_boot_js = themes::boot_script();
    let cache_control = if post.is_some() {
        // Comments and their deletion tombstones are live database state.
        // Never let an anonymous edge copy hide a write or an admin closure.
        HeaderValue::from_static("no-store")
    } else {
        HeaderValue::from_static("public, max-age=0, s-maxage=86400")
    };
    view! {
        // Default edge TTL for HTML that does not set Cache-Control itself.
        // First mention wins: pages that emit their own header before shell()
        // keep it (spire/log/feed use s-maxage=60; lifting/API use no-store).
        // Cloudflare CDN honors s-maxage when the zone Cache Rule makes HTML
        // eligible; deploy CI purges the zone so RELEASE_ID-style busting is
        // not needed.
        ((header::CACHE_CONTROL, cache_control))
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <meta name="referrer" content="strict-origin-when-cross-origin">
                <title>(title)</title>
                // Fresh HTML is already the tmux session. Rust derives this
                // tiny pre-paint allowlist from the appearance registry; only
                // a remembered alternate adds data-theme.
                <script>(Unescaped::new_unchecked(theme_boot_js))</script>
                topcoat::dev::script()
                if runtime {
                    topcoat::runtime::script()
                }
                <script
                    type="module"
                    src=(APPEARANCE_JS)
                    data-appearance-runtime=""
                    data-default-theme=(themes::DEFAULT_THEME_ID)
                    data-theme-key=(themes::THEME_STORAGE_KEY)
                ></script>
                // The account dialog's companion loads before the generic
                // modal driver so its `modal:open` listener is attached when
                // the driver opens a returned error notice on load. Neither
                // needs Topcoat's runtime; both are plain delegated modules.
                <script type="module" src=(AUTH_DIALOG_JS)></script>
                <script type="module" src=(MODALS_JS)></script>
                <script
                    type="module"
                    src=(NAVIGATION_JS)
                    data-navigation-runtime=""
                    data-hints-module=(NAVIGATION_HINTS_JS)
                ></script>
                <script
                    type="module"
                    src=(SESSION_JS)
                    data-session-runtime=""
                    data-default-theme=(themes::DEFAULT_THEME_ID)
                    data-vimium-module=(SESSION_VIMIUM_JS)
                ></script>
                <link rel="stylesheet" href=(SITE_CSS)>
                // These faces belong to oxide/night-shift, not the default
                // session. Keep their @font-face rules available for a live
                // switch without preloading their bytes on every visit.
                topcoat::font::link(font: ZILLA_SLAB, preload: false)
                topcoat::font::link(font: FIRA_SANS, preload: false)
                topcoat::font::link(font: FIRA_MONO)
                if marker_font {
                    topcoat::font::link(font: KALAM)
                }
                // preload: false — the @font-face stylesheet rides along,
                // but the woff2 bytes only download once a clown-mode stack
                // actually uses the family.
                topcoat::font::link(font: COMIC_NEUE, preload: false)
                // Hashed PNGs for browsers; app/favicon.rs serves /favicon.ico
                // for the non-HTML clients that guess the path.
                <link rel="icon" type="image/png" sizes="32x32" href=(FAVICON_32)>
                <link rel="icon" type="image/png" sizes="16x16" href=(FAVICON_16)>
                <link rel="apple-touch-icon" sizes="180x180" href=(APPLE_TOUCH_ICON)>
                if pwa {
                    // The /diary app surface (app/pwa.rs); the color matches
                    // --color-page so the standalone status bar blends in.
                    <link rel="manifest" href="/diary.webmanifest">
                    <meta name="theme-color" content="#2e3626">
                }
                if fitness_pwa {
                    <link rel="manifest" href="/fitness.webmanifest">
                    <meta name="theme-color" content="#2e3626">
                }
                <link
                    rel="alternate"
                    type="application/rss+xml"
                    title="Ben Berman — logbook"
                    href="/feed.xml"
                >
            </head>
            <body
                class="flex min-h-screen flex-col bg-page font-body text-ink"
                data-nav-hidden=(nav_hidden)
            >
                if !hide_nav {
                    <header class="mx-auto flex w-full max-w-4xl items-baseline justify-between px-5 pt-6">
                        <a
                            href="/"
                            class="font-display text-lg font-semibold text-ink no-underline hover:text-oxide"
                        >"Ben Berman"</a>
                        // Home is the logbook now; the header keeps only the
                        // two fixed rooms (the tmux windows are the full map).
                        <nav class="flex gap-6 font-meta text-sm">
                            <a href="/" class=(nav("~"))>"~"</a>
                            <a href="/resume" class=(nav("resume"))>"résumé"</a>
                        </nav>
                    </header>
                    // The phone shell: one minimal bar of pane tabs with the
                    // theme switcher at its right edge. styles/panes.css shows
                    // it below 40rem and stands the desktop chrome down; on
                    // `/` the tabs address the swipe deck's panes by fragment.
                    <nav class="pane-bar" aria-label="site panes">
                        <div class="pane-bar-tabs">
                            for tab in PANE_TABS.iter() {
                                <a
                                    class="pane-tab"
                                    data-pane-tab=(*tab)
                                    aria-current=((active_pane == Some(*tab)).then_some("page"))
                                    href=(pane_href(at_home, tab).as_str())
                                >(*tab)</a>
                            }
                        </div>
                        theme_switcher()
                    </nav>
                }
                // Rust knows the active pane before the browser runs.
                <main
                    class="mx-auto w-full max-w-4xl flex-1 px-5 pb-20"
                    data-session-title=(pane_title.as_str())
                >
                    (child)
                    if let Some(post) = post {
                        crate::app::thoughts::comments::comment_section(slug: post.slug)
                        back_link(href: "/thoughts", label: "all thoughts")
                    }
                </main>
                <footer class="mx-auto w-full max-w-4xl px-5 pb-8">
                    <div class="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-2 border-t border-hairline pt-4 font-meta text-xs text-muted">
                        <span class="flex flex-wrap gap-x-5 gap-y-2">
                            <a
                                href="https://www.linkedin.com/in/benmberman"
                                class="quiet-link"
                            >"LinkedIn"</a>
                            <a href="https://github.com/rivertam" class="quiet-link">"GitHub"</a>
                            <a
                                href="https://www.reddit.com/user/BenjiSponge"
                                class="quiet-link"
                            >"Reddit"</a>
                        </span>
                        <span>
                            (format!("entry № {:04} of {:04} · ", LOG.len(), LOG.len()))
                            "made with "
                            <a href="https://github.com/tokio-rs/topcoat" class="quiet-link">"topcoat"</a>
                        </span>
                    </div>
                    if let Some(current) = signed_in.as_ref() {
                        <p class="mt-2 text-right font-meta text-[11px] text-muted opacity-60 transition-opacity hover:opacity-100">
                            "signed in as "
                            (current.email.as_str())
                            " · "
                            if access::is_admin(&current.email) {
                                <a class="quiet-link" href="/admin">"admin"</a>
                                " · "
                            }
                            <a
                                class="quiet-link"
                                href=(login_href.as_str())
                                data-modal-open="account-dialog"
                            >"sign out"</a>
                        </p>
                    } else {
                        <p class="mt-2 text-right font-meta text-[11px] text-muted opacity-60 transition-opacity hover:opacity-100">
                            <a
                                class="quiet-link"
                                href=(login_href.as_str())
                                data-modal-open="account-dialog"
                            >"log in with google"</a>
                        </p>
                    }
                </footer>
                // The primary nav: Rust owns its windows and message surface;
                // session.js adds only client-local clock and key behavior.
                <nav class="tmux-bar" aria-label="tmux windows" data-session-bar="">
                    <a class="tmux-session" href="/">">_ bens-site"</a>
                    <div class="tmux-windows">
                        for win in windows.iter() {
                            <a
                                class="tmux-window"
                                data-session-window=""
                                aria-current=(win.current.then_some("page"))
                                href=(win.href.as_str())
                            >(win.label.as_str())</a>
                        }
                    </div>
                    <span class="tmux-status-right">
                        <span class="tmux-keys">"^a n: windows · f: follow · j/k: move"</span>
                        <span class="tmux-prefix" aria-hidden="true">"^A"</span>
                        <span class="tmux-host">"sponge"</span>
                        <span class="tmux-clock" data-session-clock=""></span>
                        if signed_in.is_some() {
                            <a
                                class="tmux-login"
                                href=(login_href.as_str())
                                data-modal-open="account-dialog"
                                aria-label="signed-in account"
                            >"signed in"</a>
                        } else {
                            <a
                                class="tmux-login"
                                href=(login_href.as_str())
                                data-modal-open="account-dialog"
                                aria-label="log in"
                            >"login"</a>
                        }
                    </span>
                    <button
                        type="button"
                        class="tmux-note"
                        data-session-message=""
                        aria-live="polite"
                        title="dismiss"
                        hidden=""
                    ></button>
                </nav>
                account_dialog(return_to: return_to.as_str())
                corner_rack()
            </body>
        </html>
    }
}

/// One account surface for the whole document, built on the generic `modal`
/// component. Identity and OAuth configuration are known while the shell
/// renders; the shared driver (`modals.js`) opens and traps the dialog, while
/// the thin auth companion (`auth-dialog.js`) keeps each `next` pointed at the
/// live URL, fragment included. `/login?next=…` remains every trigger's
/// fallback.
#[component]
async fn account_dialog(cx: &Cx, return_to: &str) -> Result {
    let current = viewer(cx);
    let configured = login_configured();
    let notice = popup_notice(cx);
    let (kicker, heading) = if current.is_some() {
        ("authenticated", "Signed in.")
    } else {
        ("sign in", "Sign in.")
    };
    let google_href = format!("/auth/google?next={}&popup=1", urlencode(return_to));
    let logout_action = format!("/logout?next={}", urlencode(return_to));
    view! {
        modal(
            id: "account-dialog",
            label: "Account",
            labelledby: "account-dialog-heading",
            open_on_load: notice.is_some(),
            // Config for the auth companion (auth-dialog.js): it finds the
            // dialog from here and reads the return/error contract, keeping the
            // OAuth and logout `next` aimed at the live URL.
            <span
                hidden=""
                data-account-config=""
                data-auth-return=(return_to)
                data-auth-error-param=(POPUP_ERROR_PARAM)
            ></span>
            <p class="auth-dialog-kicker">(kicker)</p>
            <h2 id="account-dialog-heading" class="auth-dialog-heading">(heading)</h2>
            if let Some(message) = notice {
                <p class="auth-dialog-notice" role="status">(message)</p>
            }
            if let Some(current) = current.as_ref() {
                <p class="auth-dialog-copy">
                    "You’re signed in as "
                    <span class="auth-dialog-email">(current.email.as_str())</span>
                    "."
                </p>
                <div class="auth-dialog-actions">
                    <form method="post" action=(logout_action.as_str()) data-auth-logout="">
                        <button type="submit" class="auth-dialog-primary" autofocus="">
                            "sign out"
                        </button>
                    </form>
                    if access::is_admin(&current.email) {
                        <a class="auth-dialog-secondary" href="/admin">"admin tools"</a>
                    }
                </div>
            } else {
                if configured {
                    <p class="auth-dialog-copy">
                        "Sign in with Google to join the comments or open a private page shared with you."
                    </p>
                    <div class="auth-dialog-actions">
                        <a
                            class="auth-dialog-primary"
                            href=(google_href.as_str())
                            data-auth-google=""
                            autofocus=""
                        >"continue with Google "<span aria-hidden="true">"→"</span></a>
                    </div>
                } else {
                    <p class="auth-dialog-copy">
                        "Sign-in isn’t configured in this environment."
                    </p>
                }
            }
        )
    }
}

/// The fixed bottom-right rack: the transport (visible whenever the page
/// has a music source — a whimsical theme's tune or a page band like
/// /podrick's anthem; appearance.js unhides it and paints ▶/⏸ from the real
/// state) beside the theme switcher. The volume slider slides out on
/// hover/focus and scales every sound the site makes. Music controls all
/// carry `data-music-toggle`, so the pill, the menu's ♪ row, and any page
/// chip are views of the one remembered preference.
#[component]
async fn corner_rack() -> Result {
    view! {
        <div class="corner-rack">
            <div class="band-wrap" hidden="">
                <input
                    type="range"
                    class="band-volume"
                    min="0"
                    max="100"
                    value="50"
                    aria-label="music volume"
                >
                <button
                    type="button"
                    class="band-pill"
                    data-music-toggle=""
                    aria-pressed="false"
                    title="pause / play the music"
                >"♪"</button>
            </div>
            theme_switcher()
        </div>
    }
}

/// The paint-chip rack in the corner: one row per registry entry, each row's
/// swatch dot wearing its own theme via `data-theme` (the appearance
/// stylesheets resolve tokens against the dot itself). Rust marks tmux pressed
/// because it is the attribute-less default; appearance.js adjusts that cached
/// markup only for a stored alternate.
#[component]
async fn theme_switcher() -> Result {
    view! {
        <details class="theme-dd">
            <summary aria-label="change the site theme">
                <span class="theme-dot" aria-hidden="true"></span>
                // Wrapped so the pane bar's compact copy can shed the word
                // and keep only the paint-chip dot.
                <span class="theme-dd-label">"theme"</span>
            </summary>
            <div class="theme-dd-menu" role="group" aria-label="site themes">
                for theme in themes::THEMES.iter() {
                    <button
                        type="button"
                        class="theme-option"
                        data-set-theme=(theme.id)
                        data-theme-module=(theme.module)
                        data-theme-music=(theme.music_asset)
                        data-theme-image=(theme.image_asset)
                        aria-pressed=(if theme.id == themes::DEFAULT_THEME_ID { "true" } else { "false" })
                    >
                        <span class="theme-dot" data-theme=(theme.id) aria-hidden="true"></span>
                        (theme.label)
                        if theme.whimsical {
                            // The big top marks the especially whimsical.
                            <span class="theme-whimsy" aria-hidden="true">"🎪"</span>
                            <span class="sr-only">"(especially whimsical)"</span>
                        }
                    </button>
                }
                // The especially whimsical themes carry a tune, on by
                // default; the row only appears while one is worn
                // (the active package's CSS reveals it) and is the opt-out,
                // remembered per browser.
                <button
                    type="button"
                    class="theme-option theme-music"
                    data-music-toggle=""
                    aria-pressed="false"
                    title="the big top comes with a band; silence it here"
                >
                    <span class="theme-music-note" aria-hidden="true">"♪"</span>
                    "music"
                </button>
            </div>
        </details>
    }
}
