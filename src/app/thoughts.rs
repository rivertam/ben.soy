pub(crate) mod comments;
pub mod crop_deaths;
pub mod pesky_code;
pub mod planes;
pub mod saamd;

use topcoat::{Result, router::page, view::view};

use crate::{
    components::{index_card, page_head, shell},
    content::posts::POSTS,
};

#[page("/thoughts")]
async fn thoughts() -> Result {
    view! { shell(page: "Thoughts", active: "",
        page_head(
            stamp: "log",
            title: "Thoughts",
            lede: "Thoughts of varying seriousness and length.",
        )
        <section class="mt-14 space-y-10">
            for post in POSTS.iter() {
                index_card(
                    stamp: post.date,
                    href: format!("/thoughts/{}", post.slug),
                    title: post.title,
                    teaser: post.teaser,
                )
            }
        </section>
    ) }
}
