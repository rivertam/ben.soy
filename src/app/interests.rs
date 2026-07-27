//! The interests index. Each linked interest is a standalone top-level page
//! in its own module below `app/interests/`.

mod drums;
mod felix;
mod keyboards;
pub(crate) mod lifting;
mod puzzles;
mod simulation;
pub(crate) mod spire;
mod swing;

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::page,
    view::view,
};

use super::login::viewer;
use crate::{
    components::{index_card, page_head, shell},
    content::{access, interests::INTERESTS},
};

#[page("/interests")]
async fn interests(cx: &Cx) -> Result {
    // Allowlisted hidden pages join the index for their viewers only; the
    // viewer layer keeps those personalized renders out of the CDN.
    let hidden: Vec<&access::HiddenPage> = match viewer(cx) {
        Some(current) => access::visible_pages(app_context::<Data>(cx), &current.email).await,
        None => Vec::new(),
    };
    view! {
        shell(
            title: "Interests",
            active: "interests",
            page_head(stamp: "index", title: "Interests", lede: "I contain multitudes.")
            <section class="mt-14 space-y-10">
                for interest in INTERESTS.iter() {
                    index_card(
                        stamp: interest.slug,
                        href: format!("/{}", interest.slug),
                        title: interest.title,
                        teaser: interest.teaser
                    )
                }
                for page in hidden.iter() {
                    index_card(
                        stamp: page.stamp,
                        href: page.path.to_string(),
                        title: page.title,
                        teaser: page.teaser
                    )
                }
            </section>
        )
    }
}
