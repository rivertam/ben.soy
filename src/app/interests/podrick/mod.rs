//! The `/podrick` interest page.
//!
//! Podrick himself is `podrick.rs`, a separate binary in this folder that runs
//! as its own service (`docs/podrick.md`). This module is only the page that
//! describes him, plus the small status block that proves he is alive.
//!
//! The page renders without a database, without the bot running, and without
//! any `podrick_*` row ever having been written — a bot being down should not
//! take a page down.

pub(crate) mod status;

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::page,
    view::view,
};

use crate::{
    components::{back_link, ext_link, page_head, rail_prose, rail_section, shell},
    content::interests::interest,
};

#[page("/podrick")]
async fn podrick(cx: &Cx) -> Result {
    let meta = interest("podrick");
    let summary = status::load(app_context::<Data>(cx)).await;
    let announced = match summary.posted {
        0 => "no lifts announced yet".to_string(),
        1 => "1 lift announced".to_string(),
        posted => format!("{posted} lifts announced"),
    };

    view! {
        shell(
            title: meta.title,
            active: "interests",
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)

            rail_prose(
                class: "mt-4",
                stamp: "who",
                <p>
                    "Podrick is a Discord bot I wrote for a server I'm in. He is named after "
                    ext_link(
                        class: "quiet-link",
                        href: "https://awoiaf.westeros.org/index.php/Podrick_Payne",
                        label: "Podrick Payne →"
                    )
                    ", who is loyal, useful, and does not say much. That is roughly the design brief."
                </p>
            )

            rail_prose(
                class: "mt-4",
                stamp: "job 1",
                <p>
                    "When I publish a lift here, Podrick posts it to a channel as plain text: the \
                     title, the date and duration, then every set with its load, reps, and effort, \
                     ending in a link to the permanent page. It reads like the share sheet on a \
                     workout page, because that is the point — the channel gets the workout, not a \
                     summary of it. Personal records are deliberately left out; the link has them."
                </p>
                <p>
                    "The interesting part is what he refuses to do. Podrick records a watermark the \
                     first time he runs and never announces anything older than it, so the three \
                     years of lifting history already in the archive stay out of the channel. Every \
                     announcement is claimed in the database before it is posted, so a crash \
                     halfway through, a redeploy, or two copies of him running at once still \
                     produce exactly one message."
                </p>
            )

            rail_prose(
                class: "mt-4",
                stamp: "job 2",
                <p>
                    "Not built yet. He will watch a second channel and keep some records straight \
                     in response to what gets posted there, seeded from that channel's history."
                </p>
            )

            rail_prose(
                class: "mt-4",
                stamp: "how",
                <p>
                    "No gateway connection and no bot framework — both jobs are shaped like plain \
                     REST calls, and the version that reads a channel's history is the same code \
                     that reads its newest messages. He runs as his own small service next to the \
                     site and shares its database."
                </p>
            )

            rail_section(
                class: "mt-4",
                stamp: "status",
                <p class="font-meta text-sm text-ink2">
                    (announced)
                    if let Some(latest) = summary.latest.as_ref() {
                        " · most recently "
                        <a class="quiet-link" href=(format!("/lifting/{}", latest.workout_path))>
                            "this one →"
                        </a>
                    }
                </p>
            )

            back_link(href: "/interests", label: "all interests")
        )
    }
}
