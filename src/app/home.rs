//! `~`: the front door, styled after netrw. One flat listing of everything
//! the site holds — the log, the résumé, every interest — because a
//! directory listing is the honest shape of a personal site. Granted hidden
//! pages join as dotfiles for their viewers only (the response layer keeps
//! those renders out of the CDN), and old `/interests` bookmarks land here.

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{error::redirect_permanent, page, route, uri},
    view::view,
};

use super::login::viewer;
use crate::{
    components::shell,
    content::{access, interests::INTERESTS},
};

/// True when a query string carries a key the old `/` timeline used
/// (`?kind=`, `?tag=`, `?q=`, `?page=`) and should land on `/log` instead.
fn legacy_timeline_query(query: &str) -> bool {
    query
        .split('&')
        .any(|pair| matches!(pair.split('=').next(), Some("kind" | "tag" | "q" | "page")))
}

/// One listing row: the name as netrw would print it (directories keep
/// their trailing slash, hidden pages wear a leading dot), where it goes,
/// and the registry teaser as the long-listing annotation.
struct Listing {
    name: String,
    href: String,
    blurb: &'static str,
    hidden: bool,
}

#[page("/")]
async fn home(cx: &Cx) -> Result {
    // The timeline lived at `/` until 2026-08 and its filter and pager URLs
    // travel; forward them to /log with the query intact rather than
    // quietly rendering a directory. (The query string is never decoded, so
    // it is safe inside a Location header.)
    if let Some(query) = uri(cx).query()
        && legacy_timeline_query(query)
    {
        return Err(redirect_permanent(&format!("/log?{query}")).into());
    }

    // Allowlisted hidden pages join the listing for their viewers only, the
    // way netrw shows dotfiles to people who ask.
    let current = viewer(cx);
    let granted: Vec<&access::HiddenPage> = match current.as_ref() {
        Some(current) => access::visible_pages(app_context::<Data>(cx), &current.email).await,
        None => Vec::new(),
    };

    let mut rows: Vec<Listing> = vec![
        Listing {
            name: "log/".to_string(),
            href: "/log".to_string(),
            blurb: "the logbook: essays, notes, spire wins, lifts",
            hidden: false,
        },
        Listing {
            name: "resume/".to_string(),
            href: "/resume".to_string(),
            blurb: "what I do professionally",
            hidden: false,
        },
    ];
    rows.extend(INTERESTS.iter().map(|interest| Listing {
        name: format!("{}/", interest.slug),
        href: format!("/{}", interest.slug),
        blurb: interest.teaser,
        hidden: false,
    }));
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
        shell(title: "", active: "~",
        <section class="netrw mt-12 font-meta text-[13px] sm:text-sm">
            <h1 class="sr-only">"Ben Berman"</h1>
            // The banner: vim comment lines, muted like the plugin they
            // imitate. Truncation over wrapping — a clipped rule is still a
            // rule, a wrapped one is debris.
            <header class="text-muted" aria-hidden="true">
                <p class="overflow-hidden text-ellipsis whitespace-pre">"\" ============================================================"</p>
                <p class="overflow-hidden text-ellipsis whitespace-pre">"\" Welcome to my site!"</p>
                <p class="overflow-hidden text-ellipsis whitespace-pre">"\"   Owned by       Ben Berman (software developer, New York)"</p>
                <p class="overflow-hidden text-ellipsis whitespace-pre">"\"   Quick Help:    "<span class="netrw-keys">"j/k:move  <cr>:open  f:follow  "</span>"clicking also works"</p>
                <p class="overflow-hidden text-ellipsis whitespace-pre">"\" ============================================================"</p>
            </header>
            <div class="mt-1 overflow-hidden text-ellipsis whitespace-pre text-muted">
                "\"   Session:       "
                if current.is_some() {
                    <a class="netrw-login" href="/login" aria-label="signed-in account">"signed in"</a>
                    <form method="post" action="/logout" class="inline">
                        " · "
                        <button type="submit" class="netrw-logout cursor-pointer">"logout"</button>
                    </form>
                } else {
                    <a class="netrw-login" href="/login" aria-label="log in">"login"</a>
                }
            </div>
            <ul class="mt-1 space-y-1">
                <li class="text-muted" title="there is no up from here">"../"</li>
                for row in rows.iter() {
                    <li
                        class="netrw-row relative"
                        data-rail-item=""
                        data-rail-href=(row.href.as_str())
                    >
                        <a
                            class=(if row.hidden {
                                "group flex items-baseline gap-x-5 no-underline opacity-75"
                            } else {
                                "group flex items-baseline gap-x-5 no-underline"
                            })
                            href=(row.href.as_str())
                        >
                            <span class=(if row.name.ends_with('/') {
                                "w-36 shrink-0 text-oxide group-hover:underline"
                            } else {
                                "w-36 shrink-0 text-ink group-hover:underline"
                            })>(row.name.as_str())</span>
                            <span class="min-w-0 truncate text-muted">(row.blurb)</span>
                        </a>
                    </li>
                }
            </ul>
        </section>
        )
    }
}

/// The old interests index, folded into the listing above: bookmarks land
/// on `~`. (The per-page `/interests/{slug}` redirects live with their
/// pages.)
#[route(GET "/interests")]
async fn legacy_interests() -> Result {
    Err(redirect_permanent("/").into())
}

#[cfg(test)]
mod tests {
    use super::legacy_timeline_query;

    #[test]
    fn legacy_timeline_queries_are_recognized() {
        assert!(legacy_timeline_query("kind=note"));
        assert!(legacy_timeline_query("page=2"));
        assert!(legacy_timeline_query("tag=spire&q=how%20bad"));
        assert!(legacy_timeline_query("utm_source=x&page=3"));
        assert!(!legacy_timeline_query("utm_source=x"));
        assert!(!legacy_timeline_query("unkind=true"));
        assert!(!legacy_timeline_query(""));
    }
}
