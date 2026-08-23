//! Interest registry, mirroring `posts.rs`. Each interest is a standalone
//! page module under `app/interests/`; this list is the single source of
//! truth for its slug (top-level route `/{slug}`, nav label, rail stamp),
//! display title, and teaser (the `~` listing's annotation, doubling as the
//! page's lede). The `~` listing, the tmux windows, and the 404's route
//! list all derive from here — adding an interest means one entry here plus
//! the page module.

pub struct Interest {
    pub slug: &'static str,
    pub title: &'static str,
    pub teaser: &'static str,
}

pub static INTERESTS: [Interest; 6] = [
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
        slug: "fitness",
        title: "Fitness",
        teaser: "Lifting and running, logged together",
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
];

/// An interest page's own registry entry. Panics on a slug not in the
/// registry — pages pass literals, so a typo surfaces on the first render.
pub fn interest(slug: &str) -> &'static Interest {
    INTERESTS
        .iter()
        .find(|i| i.slug == slug)
        .unwrap_or_else(|| panic!("unknown interest slug: {slug}"))
}
