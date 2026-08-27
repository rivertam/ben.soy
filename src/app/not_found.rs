//! The 404, engineered with more care than a 404 deserves — in the family
//! tradition of upstream patches about meaningful error messages. The server
//! runs edit distance between the requested path and every real route, then
//! declines to presume.

use topcoat::{
    Result,
    context::Cx,
    router::{StatusCode, page, request::uri},
    view::{component, view},
};

use crate::{components::shell, content::routes::site_routes};

/// Plain Levenshtein, two rolling rows. The route inventory is small enough
/// that this is not the expensive part of the request.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[page("/{*rest}")]
async fn not_found(cx: &Cx) -> Result {
    view! { not_found_page(requested: uri(cx).path()) }
}

/// The full 404 document. Also rendered by hidden pages for signed-in
/// visitors who aren't on the page's allowlist, so denial is
/// indistinguishable from absence.
#[component]
pub async fn not_found_page(requested: &str) -> Result {
    let routes = site_routes();

    let (distance, nearest) = routes
        .iter()
        .map(|route| {
            (
                edit_distance(&requested.to_lowercase(), &route.to_lowercase()),
                route.clone(),
            )
        })
        .min_by_key(|(distance, _)| *distance)
        .expect("routes is never empty");

    // Close enough to be a typo: name the route and the distance, but
    // redirecting would have been presumptuous.
    let suggestion = (distance <= 3).then_some(nearest);
    let edits = if distance == 1 {
        "1 edit".to_string()
    } else {
        format!("{distance} edits")
    };
    let suggestion_tail =
        format!(" is {edits} away; redirecting there would have been presumptuous.");

    view! {
        shell(
            page: "404",
            active: "",
            (StatusCode::NOT_FOUND)
            <section class="mt-16 sm:mt-24">
                <header class="rail-row">
                    <p class="rail-stamp">"404"</p>
                    <div class="min-w-0">
                        <h1 class="font-display text-4xl font-bold tracking-tight">
                            "No such page."
                        </h1>
                        <p class="mt-4 font-meta text-sm text-ink2">
                            "GET "
                            (requested)
                            " — nothing at that path."
                        </p>
                        if let Some(path) = suggestion {
                            <p class="mt-3 max-w-prose text-ink2">
                                <a class="oxlink" href=(path.as_str())>(path.as_str())</a>
                                (suggestion_tail.as_str())
                            </p>
                        }
                        <p class="mt-8 rail-stamp rail-stamp-label">"the index"</p>
                        <ul class="mt-2 space-y-1 font-meta text-sm">
                            for route in routes.iter() {
                                <li>
                                    <a class="quiet-link" href=(route.as_str())>
                                        (route.as_str())
                                    </a>
                                </li>
                            }
                        </ul>
                    </div>
                </header>
            </section>
        )
    }
}
