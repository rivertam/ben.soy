//! The document shell: fonts, head, nav, footer. Every page renders through
//! this.

use benjisponge::data::Data;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    font::{Font, fontsource::fontsource_font},
    router::{HeaderValue, header},
    view::{View, component, view},
};

use crate::app::login::viewer;
use crate::components::link_label;
use crate::content::{access, interests::INTERESTS, logbook::LOG};

pub const ZILLA_SLAB: Font = fontsource_font!(ZILLA_SLAB, host: Asset);
pub const FIRA_SANS: Font = fontsource_font!(FIRA_SANS, host: Asset);
pub const FIRA_MONO: Font = fontsource_font!(FIRA_MONO, host: Asset);
const ANALYTICS_JS: Asset = asset!("./analytics.js");
const FAVICON_16: Asset = asset!("./favicon/favicon-16.png");
const FAVICON_32: Asset = asset!("./favicon/favicon-32.png");
const APPLE_TOUCH_ICON: Asset = asset!("./favicon/apple-touch-icon.png");

/// The full document: every page renders through this, so every page owns its
/// title. Pages invoke it as markup with the page content as trailing children:
/// `view! { shell(title: "…", active: "…", <p>"…"</p>) }`.
///
/// `title` is the bare page title — the shell appends "— Ben Berman" itself;
/// pass `""` for the homepage, whose title is just the name.
///
/// `active` names the nav item the page lives under — `"log"`, `"resume"`,
/// `"interests"`, or `""` for none — and gets the oxide underline.
///
/// `hide_nav` removes the header for an immersive, self-contained page.
///
/// `runtime` controls Topcoat's browser runtime. It defaults on for existing
/// pages; fully server-rendered pages can opt out and ship no production JS.
///
/// `analytics` controls the first-party tracker. It is disabled on the 404 so
/// arbitrary requested paths can never become public dashboard entries.
///
/// Signed-in viewers get two quiet extras: their allowlisted hidden pages
/// join the interests dropdown, and a barely-there "signed in" line sits at
/// the footer's bottom right. Both personalize the HTML, which is why
/// `response_layer.rs` forces `private, no-store` whenever the viewer cookie
/// rides the request — the header below only governs anonymous renders.
#[component]
pub async fn shell(
    cx: &Cx,
    title: &str,
    active: &str,
    #[default(false)] hide_nav: bool,
    #[default(true)] runtime: bool,
    #[default(true)] analytics: bool,
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
    view! {
        // Default edge TTL for HTML that does not set Cache-Control itself.
        // First mention wins: pages that emit their own header before shell()
        // keep it (spire/home/feed use s-maxage=60; lifting/API use no-store).
        // Cloudflare CDN honors s-maxage when the zone Cache Rule makes HTML
        // eligible; deploy CI purges the zone so RELEASE_ID-style busting is
        // not needed.
        ((header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=0, s-maxage=86400")))
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <meta name="referrer" content="strict-origin-when-cross-origin">
                <title>(title)</title>
                topcoat::dev::script()
                if runtime {
                    topcoat::runtime::script()
                }
                if analytics {
                    <script defer="" src=(ANALYTICS_JS)></script>
                }
                <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
                topcoat::font::link(font: ZILLA_SLAB)
                topcoat::font::link(font: FIRA_SANS)
                topcoat::font::link(font: FIRA_MONO)
                // Hashed PNGs for browsers; app/favicon.rs serves /favicon.ico
                // for the non-HTML clients that guess the path.
                <link rel="icon" type="image/png" sizes="32x32" href=(FAVICON_32)>
                <link rel="icon" type="image/png" sizes="16x16" href=(FAVICON_16)>
                <link rel="apple-touch-icon" sizes="180x180" href=(APPLE_TOUCH_ICON)>
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
                        <nav class="flex gap-6 font-meta text-sm">
                            <a href="/" class=(nav("log"))>"log"</a>
                            <a href="/resume" class=(nav("resume"))>"résumé"</a>
                            <details class="nav-dd">
                                <summary class=(nav("interests"))>"interests"</summary>
                                <div class="nav-dd-menu">
                                    <a class="quiet-link" href="/interests">
                                        link_label(label: "all interests →")
                                    </a>
                                    for interest in INTERESTS.iter() {
                                        <a
                                            class="quiet-link"
                                            href=(format!("/{}", interest.slug))
                                        >(interest.slug)</a>
                                    }
                                    for hidden in hidden_pages.iter() {
                                        <a class="quiet-link" href=(hidden.path)>(hidden.stamp)</a>
                                    }
                                </div>
                            </details>
                        </nav>
                    </header>
                }
                <main class="mx-auto w-full max-w-4xl flex-1 px-5 pb-20">(child)</main>
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
                            <a href="/analytics" class="quiet-link">"Analytics"</a>
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
                    }
                </footer>
            </body>
        </html>
    }
}
