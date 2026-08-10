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

use crate::app::login::viewer;
use crate::content::{access, interests::INTERESTS, logbook::LOG, themes};

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
const THEME_JS: Asset = asset!("./theme.js");
/// Clown mode's band: a Pixabay circus track (their license permits this
/// use without attribution). Fetched only when the tune actually starts —
/// theme.js reads the hashed URL off the ♪ row's data attribute.
const CIRCUS_MP3: Asset = asset!("./circus.mp3");
/// Felix mode's chaser: Felix himself, cut out of the 2023 sprint photo,
/// who trails the tennis-ball cursor and eventually catches it. Same
/// data-attribute delivery as the mp3; only fetched in felix mode.
const FELIX_CHASER: Asset = asset!("./felix-chaser.webp");
const FAVICON_16: Asset = asset!("./favicon/favicon-16.png");
const FAVICON_32: Asset = asset!("./favicon/favicon-32.png");
const APPLE_TOUCH_ICON: Asset = asset!("./favicon/apple-touch-icon.png");

/// One window in the tmux theme's status bar: a precomputed `"3 felix"`
/// label, its destination, and whether the request path lives inside it.
struct TmuxWindow {
    label: String,
    href: String,
    current: bool,
}

/// The tmux theme's windows: the site's full flat map, mirroring the `~`
/// listing (~, the log, the résumé, each interest, any granted hidden
/// pages) — a superset of the header's three fixed links — numbered in
/// render order.
/// The current window is the longest matching path prefix — `/lifting/log`
/// lights `lifting` while `/thoughts/anything` stays home at `~`.
fn tmux_windows(path: &str, hidden_pages: &[&'static access::HiddenPage]) -> Vec<TmuxWindow> {
    let mut windows: Vec<(String, String)> = vec![
        ("~".to_string(), "/".to_string()),
        ("log".to_string(), "/log".to_string()),
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
        .map(|(index, (name, href))| TmuxWindow {
            label: format!("{index} {name}"),
            href,
            current: index == active,
        })
        .collect()
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
/// `pwa` links the /diary app manifest and its status-bar color so the page
/// is installable (app/pwa.rs serves the pieces). Only the admin-only diary
/// pages set it; the flag renders no viewer data, but keeping it off
/// everywhere else keeps the public site from advertising an install.
///
/// `marker_font` loads Kalam for the handwritten caption on the dog-age post.
/// It defaults off so the extra face does not ride along on every page.
///
/// Signed-in viewers get two quiet extras: their allowlisted hidden pages
/// join the tmux windows (and the `~` listing renders them as dotfiles),
/// and a barely-there "signed in" line replaces
/// the footer's login link. Both personalize the HTML, which is why
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
    // The tmux theme's status bar rides along in every render (the HTML is
    // cached theme-blind; themes.css reveals it only under
    // [data-theme="tmux"], where it replaces the header as the nav).
    let windows = tmux_windows(uri(cx).path(), &hidden_pages);
    let pane_title = windows
        .iter()
        .find(|win| win.current)
        .map(|win| win.label.clone())
        .unwrap_or_default();
    view! {
        // Default edge TTL for HTML that does not set Cache-Control itself.
        // First mention wins: pages that emit their own header before shell()
        // keep it (spire/log/feed use s-maxage=60; lifting/API use no-store).
        // Cloudflare CDN honors s-maxage when the zone Cache Rule makes HTML
        // eligible; deploy CI purges the zone so RELEASE_ID-style busting is
        // not needed.
        ((header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=0, s-maxage=86400")))
        <!DOCTYPE html>
        <html lang="en" data-theme="tmux">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <meta name="referrer" content="strict-origin-when-cross-origin">
                <title>(title)</title>
                // The shell ships already attached: tmux is the default worn
                // by fresh viewers (and the no-JS fallback). This swaps in a
                // stored choice before the stylesheet arrives so first paint
                // is in the right costume (content/themes.rs owns the script;
                // theme.js handles the clicks).
                <script>(Unescaped::new_unchecked(themes::THEME_BOOT_JS))</script>
                topcoat::dev::script()
                if runtime {
                    topcoat::runtime::script()
                }
                <script defer="" src=(THEME_JS)></script>
                <link rel="stylesheet" href=(SITE_CSS)>
                topcoat::font::link(font: ZILLA_SLAB)
                topcoat::font::link(font: FIRA_SANS)
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
                    <meta name="theme-color" content="#f4f5f7">
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
                        // The whole flat map of the site lives at `~`; the
                        // header keeps only the three fixed rooms.
                        <nav class="flex gap-6 font-meta text-sm">
                            <a href="/" class=(nav("~"))>"~"</a>
                            <a href="/log" class=(nav("log"))>"log"</a>
                            <a href="/resume" class=(nav("resume"))>"résumé"</a>
                        </nav>
                    </header>
                }
                // data-tmux-title is the pane label the tmux theme floats
                // on the content border; inert everywhere else.
                <main
                    class="mx-auto w-full max-w-4xl flex-1 px-5 pb-20"
                    data-tmux-title=(pane_title.as_str())
                >(child)</main>
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
                        <form
                            method="post"
                            action="/logout"
                            class="mt-2 text-right font-meta text-[11px] text-muted opacity-60 transition-opacity hover:opacity-100"
                        >
                            "signed in as "
                            (current.email.as_str())
                            " · "
                            if access::is_admin(&current.email) {
                                <a class="quiet-link" href="/admin">"admin"</a>
                                " · "
                            }
                            <button type="submit" class="quiet-link cursor-pointer">"sign out"</button>
                        </form>
                    } else {
                        <p class="mt-2 text-right font-meta text-[11px] text-muted opacity-60 transition-opacity hover:opacity-100">
                            <a class="quiet-link" href="/login">"log in with google"</a>
                        </p>
                    }
                </footer>
                // The tmux theme's status bar: session badge, numbered
                // windows, keys legend, clock. Display: none under every
                // other theme; theme.js paints the clock and the armed
                // prefix chip, and drives ctrl-a n/p/0-9/l off the
                // data-tmux-window hooks (themes.rs tests pin the contract).
                <nav class="tmux-bar" aria-label="tmux windows" data-tmux-bar="">
                    <a class="tmux-session" href="/">">_ bens-site"</a>
                    <div class="tmux-windows">
                        for win in windows.iter() {
                            <a
                                class="tmux-window"
                                data-tmux-window=""
                                aria-current=(win.current.then_some("page"))
                                href=(win.href.as_str())
                            >(win.label.as_str())</a>
                        }
                    </div>
                    <span class="tmux-status-right">
                        <span class="tmux-keys">"^a n: windows · f: follow · j/k: move"</span>
                        <span class="tmux-prefix" aria-hidden="true">"^A"</span>
                        <span class="tmux-host">"sponge"</span>
                        <span class="tmux-clock" data-tmux-clock=""></span>
                    </span>
                </nav>
                corner_rack()
            </body>
        </html>
    }
}

/// The fixed bottom-right rack: the transport (visible whenever the page
/// has a music source — a whimsical theme's tune or a page band like
/// /podrick's anthem; theme.js unhides it and paints ▶/⏸ from the real
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
/// swatch dot wearing its own theme via `data-theme` (themes.css resolves
/// tokens against the dot itself). Server renders every row unpressed —
/// the HTML is cached for anonymous viewers, so the live choice is applied
/// client-side: the boot script sets `data-theme` before paint and theme.js
/// corrects `aria-pressed` on load.
#[component]
async fn theme_switcher() -> Result {
    view! {
        <details class="theme-dd">
            <summary aria-label="change the site theme">
                <span class="theme-dot" aria-hidden="true"></span>
                "theme"
            </summary>
            <div class="theme-dd-menu" role="group" aria-label="site themes">
                for theme in themes::THEMES.iter() {
                    <button
                        type="button"
                        class="theme-option"
                        data-set-theme=(theme.id)
                        aria-pressed="false"
                        title=(theme.blurb)
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
                // (themes.css reveals it) and is the opt-out, remembered
                // per browser.
                <button
                    type="button"
                    class="theme-option theme-music"
                    data-music-toggle=""
                    data-clown-tune=(CIRCUS_MP3)
                    data-felix-chaser=(FELIX_CHASER)
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
