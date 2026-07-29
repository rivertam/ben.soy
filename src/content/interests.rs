//! Interest registry, mirroring `posts.rs`. Each interest is a standalone
//! page module under `app/interests/`; this list is the single source of
//! truth for its slug (top-level route `/{slug}`, nav label, rail stamp),
//! display title, and teaser (the index card copy, doubling as the page's
//! lede). The nav dropdown, the interests index, and the 404's route list
//! all derive from here — adding an interest means one entry here plus the
//! page module.

pub struct Interest {
    pub slug: &'static str,
    pub title: &'static str,
    pub teaser: &'static str,
}

pub static INTERESTS: [Interest; 8] = [
    Interest {
        slug: "felix",
        title: "Felix",
        teaser: "I have a dog named Felix (cute, loud)",
    },
    Interest {
        slug: "swing",
        title: "Swing dancing",
        teaser: "I am an intermediate swing dancer",
    },
    Interest {
        slug: "lifting",
        title: "Lifting",
        teaser: "I lift weights pretty often. Click on this one to learn how often!",
    },
    Interest {
        slug: "keyboards",
        title: "Keyboards",
        teaser: "Big fan of dactyls, dactyl manuform, and split-columnar keyboards. Currently on \
                 a glove80.",
    },
    Interest {
        slug: "spire",
        title: "Slay the Spire",
        teaser: "I play a lot of Slay the Spire (currently, Slay the Spire 2)",
    },
    Interest {
        slug: "drums",
        title: "Drums",
        teaser: "Mediocre drummer. Recording turns out to be much harder than playing.",
    },
    Interest {
        slug: "simulation",
        title: "Housing Market Simulation",
        teaser: "I once spent a bunch of time working on an ECS/actor framework for \
                 react-three-fiber with the intention of creating some interesting visuals about \
                 the housing market.",
    },
    Interest {
        slug: "puzzles",
        title: "Crosswords",
        teaser: "I built a little TUI to solve crosswords for fun once",
    },
];

/// An interest page's own registry entry. Panics on a slug not in the
/// registry — pages pass literals, so a typo surfaces on the first render.
pub fn interest(slug: &str) -> &'static Interest {
    INTERESTS
        .iter()
        .find(|i| i.slug == slug)
        .unwrap_or_else(|| panic!("unknown interest slug: {slug}"))
}
