//! A hidden page: not in `INTERESTS`, not in `site_routes()`, so the nav,
//! indexes, feed, and 404 never mention it publicly. Its one listing is
//! `access::HIDDEN_PAGES`, which the tmux windows and the `~` listing
//! render for allowlisted viewers only. Signed-out visitors get bounced to
//! `/login`; signed-in visitors who aren't allowlisted for it get the real
//! 404. The `no-store` header goes out before `shell()` on every variant —
//! belt for the viewer layer's suspenders, and the reason a future cookieless
//! variant of this page could never be edge-cached for a day.

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{HeaderValue, error::redirect, header, page},
    view::view,
};

use crate::components::{page_head, shell};
use crate::content::access::may_view;

use super::login::viewer;
use super::not_found::not_found_page;

#[page("/motorcycles")]
async fn motorcycles(cx: &Cx) -> Result {
    let Some(viewer) = viewer(cx) else {
        return Err(redirect("/login?next=%2Fmotorcycles").into());
    };
    if !may_view(app_context::<Data>(cx), &viewer.email, "/motorcycles").await {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
            not_found_page(requested: "/motorcycles")
        };
    }
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            title: "Motorcycles",
            active: "",
            runtime: false,
            page_head(
                stamp: "motorcycles",
                title: "Motorcycles",
                lede: "The garage log. You're seeing this because I decided you should."
            )
            <section class="mt-8 max-w-prose space-y-4 text-ink2">
                <p>
                    "Nothing here yet — this page exists so the door has a room "
                    "behind it. Bikes, routes, and wrenching notes land here next."
                </p>
            </section>
        )
    }
}
