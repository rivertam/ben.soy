use topcoat::{
    Result,
    router::{error::redirect_permanent, page, route},
    view::{component, view},
};

use crate::{
    components::{
        back_link, ext_link, inline_popover, link_label, page_head, rail_prose, rail_section,
        shell, video_card,
    },
    content::interests::interest,
};

#[component]
async fn map_generator_citation() -> Result {
    view! {
        <span class="inline-popover-preview">
            "A procedural city map generator, created for artistic purposes."
        </span>
        ext_link(
            class: "quiet-link",
            href: "https://github.com/ProbableTrain/MapGenerator",
            label: "repo →"
        )
    }
}

#[component]
async fn ecs_citation() -> Result {
    view! {
        <span class="inline-popover-preview">
            "Entity–component system: simulation state as entities with typed component data."
        </span>
        <span class="inline-popover-detail">
            "Properties of entities are typically stored in big, co-located arrays, allowing \
             disparate systems to run independently (parallelized and cache-optimal). \
             Effectively an authoring layer to give struct-of-arrays performance with \
             array-of-structs semantics."
        </span>
        ext_link(
            class: "quiet-link",
            href: "https://en.wikipedia.org/wiki/Entity_component_system",
            label: "Wikipedia →"
        )
    }
}

#[component]
async fn react_three_fiber_citation() -> Result {
    view! {
        <span class="inline-popover-preview">
            "React renderer for three.js — how the simulation was meant to be visualized."
        </span>
        ext_link(
            class: "quiet-link",
            href: "https://docs.pmnd.rs/react-three-fiber/getting-started/introduction",
            label: "docs →"
        )
    }
}

#[page("/simulation")]
async fn simulation() -> Result {
    let meta = interest("simulation");
    view! {
        shell(
            page: meta.title,
            active: "",
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)
            rail_prose(
                stamp: "backstory",
                <p>
                    "In 2023, I got into a little debate with my brother. I had said something \
                 like \"The existence and legality of renting property (i.e. landlords) \
                 raises property values\". He said something like \"You're way too sure, \
                 given the potential second- and third-order effects.\""
                </p>
                <p>
                    "I had this thought of \"this would be easy to simulate\", which I have \
                 learned is a dumb, untrue thought on both a literal and philosophical \
                 level."
                </p>
                <p>
                    "I explored many possibilities, forked from "
                    inline_popover(
                        id: "map-generator-cite",
                        label: "ProbablyTrain/MapGenerator",
                        map_generator_citation()
                    )
                    ", and played around with agentic LLMs for development for the first time."
                </p>
                <p>
                    "One of the biggest through-lines here was to create an "
                    inline_popover(id: "ecs-cite", label: "ECS", ecs_citation())
                    "-style state management system for React (and therefore "
                    inline_popover(
                        id: "react-three-fiber-cite",
                        label: "react-three-fiber",
                        react_three_fiber_citation()
                    )
                    ") with co-located memory supporting a many-entity simulation. One of the \
                 cool ideas behind making it ECS-style would be the potential to use \
                 workers to do ("
                    <em>"gasp"</em>
                    ") multi-threaded simulations/gaming in \
                 JavaScript."
                </p>
            )
            rail_section(
                stamp: "footage",
                <div class="flex flex-wrap gap-5">
                    video_card(youtube_id: "Bcd_9LvUr-8", label: "the video →")
                </div>
            )
            rail_section(
                class: "mt-6",
                stamp: "links",
                <p class="flex flex-wrap gap-x-4 gap-y-1 font-meta text-sm">
                    <a class="oxlink" href="https://github.com/rivertam/City">
                        link_label(label: "the repo →")
                    </a>
                </p>
            )
            rail_section(
                class: "mt-6",
                stamp: "What's the state?",
                <p class="flex flex-wrap gap-x-4 gap-y-1 font-meta text-sm">
                    "Abandoned, in part due to finding a job."
                </p>
            )
            rail_section(
                class: "mt-6",
                stamp: "What did I learn?",
                <ul class="flex flex-wrap gap-x-4 gap-y-1 font-meta text-sm">
                    <li>
                        "Memory-oriented performance improvements in the JavaScript runtime are quite counter-intuitive."
                    </li>
                    <li>
                        "Chaotic systems aren't well-represented by discrete data and decisions such as actors. Shout out to "
                        inline_popover(
                            id: "chaos-book-cite",
                            label: "Chaos by James Gleick",
                            <p>
                                <cite>
                                    "Gleick, James. "
                                    <em>"Chaos: Making a New Science"</em>
                                    ". Viking, 1987."
                                </cite>
                            </p>
                            ext_link(
                                class: "quiet-link",
                                href: "https://www.penguinrandomhouse.com/books/321477/chaos-by-james-gleick/",
                                label: "publisher →"
                            )
                        )
                        "."
                    </li>
                </ul>
            )
            back_link(href: "/", label: "~")
        )
    }
}

#[route(GET "/interests/simulation")]
async fn legacy_simulation() -> Result {
    Err(redirect_permanent("/simulation").into())
}
