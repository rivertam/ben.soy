use topcoat::{
    Result,
    router::{error::redirect_permanent, page, route},
    view::view,
};

use crate::{
    components::{back_link, link_label, page_head, rail_prose, rail_section, shell},
    content::interests::interest,
};

#[page("/puzzles")]
async fn puzzles() -> Result {
    let meta = interest("puzzles");
    view! {
        shell(
            page: meta.title,
            active: "",
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)
            rail_prose(
                stamp: "",
                <p>
                    "
                It's a Rust/wasm crate for parsing and manipulating .puz files,
                and then a TypeScript wrapper that makes it a TUI.
            "
                </p>
                <p>
                    "
                To be abundantly clear, it doesn't solve the puzzles. It's just a
                client to solve them. I think my brother made a puzzle and I wanted
                to solve it on the TUI, so I built this before solving it.
            "
                </p>
            )
            rail_section(
                class: "mt-6",
                stamp: "links",
                <p class="flex flex-wrap gap-x-4 gap-y-1 font-meta text-sm">
                    <a class="oxlink" href="https://github.com/rivertam/puzuzu">
                        link_label(label: "puzuzu →")
                    </a>
                </p>
            )
            back_link(href: "/", label: "~")
        )
    }
}

#[route(GET "/interests/puzzles")]
async fn legacy_puzzles() -> Result {
    Err(redirect_permanent("/puzzles").into())
}
