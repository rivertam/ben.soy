//! The logbook: the site's master feed, newest first. Adding an entry here is
//! how content gets published — the homepage timeline and `/feed.xml` both
//! derive from this array, so a new entry ships to both on the next deploy.
//!
//! Serial numbers count from the oldest entry: entry at index `i` is
//! `№ {LOG.len() - i}` (zero-padded to four digits, see [`serial`]).

/// One logbook entry. The variants render differently (card / pull-quote /
/// one-liner) but share a date and tags for filtering.
pub enum Entry {
    /// A full post, living at `/thoughts/{slug}`.
    Essay {
        date: &'static str,
        title: &'static str,
        teaser: &'static str,
        slug: &'static str,
        tags: &'static [&'static str],
    },
    /// A short thought, whose permalink is the post page at `/thoughts/{slug}`.
    Note {
        date: &'static str,
        body: &'static str,
        source: &'static str,
        slug: &'static str,
        tags: &'static [&'static str],
    },
    /// A one-line status: `[stamp] label · body link_label`.
    Update {
        date: &'static str,
        stamp: &'static str,
        label: &'static str,
        body: &'static str,
        href: &'static str,
        link_label: &'static str,
        tags: &'static [&'static str],
    },
}

/// An entry's kind, for the homepage's `?kind=` filter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Essay,
    Note,
    Update,
}

impl Entry {
    pub fn date(&self) -> &'static str {
        match self {
            Entry::Essay { date, .. } | Entry::Note { date, .. } | Entry::Update { date, .. } => {
                date
            }
        }
    }

    pub fn tags(&self) -> &'static [&'static str] {
        match self {
            Entry::Essay { tags, .. } | Entry::Note { tags, .. } | Entry::Update { tags, .. } => {
                tags
            }
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Entry::Essay { .. } => Kind::Essay,
            Entry::Note { .. } => Kind::Note,
            Entry::Update { .. } => Kind::Update,
        }
    }
}

/// The serial stamp for the entry at `index`: newest first means the top
/// entry carries the highest number.
pub fn serial(index: usize) -> String {
    format!("№ {:04}", LOG.len() - index)
}

pub static LOG: [Entry; 7] = [
    Entry::Essay {
        date: "2026-07-12",
        title: "How bad are planes?",
        teaser: "Why I don't generally take planes for leisure",
        slug: "how-bad-are-planes",
        tags: &["climate", "planes"],
    },
    Entry::Update {
        date: "2026-06-28",
        stamp: "pr",
        label: "keyboards",
        body: "TypeRacer now has me at 117wpm.",
        href: "/keyboards",
        link_label: "keyboards →",
        tags: &["keyboards"],
    },
    Entry::Update {
        date: "2026-05-19",
        stamp: "footage",
        label: "drums",
        body: "New cover on tape:",
        href: "https://www.youtube.com/watch?v=8lrjsP1KWrY",
        link_label: "Manchester Orchestra ↗",
        tags: &["music"],
    },
    Entry::Note {
        date: "2025-08-14",
        body: "I'm so glad AI can handle all that pesky code for me so I can focus on what I \
               truly love: navigating endless chains of SSO sign-ins followed by dashboards to \
               manage settings and secrets in different environments ❤️",
        source: "originally a LinkedIn post",
        slug: "pesky-code",
        tags: &["ai"],
    },
    Entry::Update {
        date: "2025-06-02",
        stamp: "win",
        label: "spire",
        body: "Ascension 20, with an annotated run synopsis.",
        href: "/spire",
        link_label: "spire →",
        tags: &["games"],
    },
    Entry::Update {
        date: "2025-04-11",
        stamp: "shipped",
        label: "puzzles",
        body: "A Rust crossword engine — .puz files open in the terminal now.",
        href: "/puzzles",
        link_label: "puzzles →",
        tags: &["rust", "games"],
    },
    Entry::Update {
        date: "2024-11-08",
        stamp: "shipped",
        label: "simulation",
        body: "Procedural cities with opinionated residents, running Schelling-style agents.",
        href: "/simulation",
        link_label: "simulation →",
        tags: &["toys"],
    },
];

/// The homepage filter row's fixed tag chips. `spire` and `fitness` filter
/// dynamic timeline items (wins and published lifts); the rest match curated
/// logbook entry tags.
pub static FILTER_TAGS: [&str; 8] = [
    "rust",
    "ai",
    "climate",
    "music",
    "keyboards",
    "games",
    "spire",
    "fitness",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::posts::POSTS;

    fn iso_date(date: &str) -> bool {
        let bytes = date.as_bytes();
        date.len() == 10
            && bytes.iter().enumerate().all(|(i, b)| match i {
                4 | 7 => *b == b'-',
                _ => b.is_ascii_digit(),
            })
    }

    #[test]
    fn dates_are_iso_and_strictly_newest_first() {
        for entry in LOG.iter() {
            assert!(iso_date(entry.date()), "bad date: {}", entry.date());
        }
        for pair in LOG.windows(2) {
            assert!(
                pair[0].date() > pair[1].date(),
                "not strictly newest-first: {} then {}",
                pair[0].date(),
                pair[1].date()
            );
        }
    }

    #[test]
    fn copy_fields_are_non_empty() {
        for entry in LOG.iter() {
            match entry {
                Entry::Essay {
                    title,
                    teaser,
                    slug,
                    ..
                } => {
                    assert!(!title.is_empty() && !teaser.is_empty() && !slug.is_empty());
                }
                Entry::Note {
                    body, source, slug, ..
                } => {
                    assert!(!body.is_empty() && !source.is_empty() && !slug.is_empty());
                }
                Entry::Update {
                    stamp,
                    label,
                    body,
                    href,
                    link_label,
                    ..
                } => {
                    assert!(
                        !stamp.is_empty()
                            && !label.is_empty()
                            && !body.is_empty()
                            && !href.is_empty()
                            && !link_label.is_empty()
                    );
                }
            }
        }
    }

    #[test]
    fn tags_are_lowercase_ascii_and_present() {
        for entry in LOG.iter() {
            assert!(!entry.tags().is_empty(), "{} has no tags", entry.date());
            for tag in entry.tags() {
                assert!(
                    tag.chars().all(|c| c.is_ascii_lowercase()),
                    "tag not lowercase ascii: {tag}"
                );
            }
        }
    }

    #[test]
    fn essay_and_note_slugs_exist_in_posts() {
        for entry in LOG.iter() {
            let slug = match entry {
                Entry::Essay { slug, .. } | Entry::Note { slug, .. } => slug,
                Entry::Update { .. } => continue,
            };
            assert!(
                POSTS.iter().any(|p| p.slug == *slug),
                "no post for slug {slug}"
            );
        }
    }

    #[test]
    fn update_hrefs_are_internal_or_https() {
        for entry in LOG.iter() {
            if let Entry::Update { href, .. } = entry {
                assert!(
                    href.starts_with('/') || href.starts_with("https://"),
                    "bad href: {href}"
                );
            }
        }
    }

    #[test]
    fn every_filter_tag_matches_an_entry() {
        for tag in FILTER_TAGS.iter() {
            let curated = LOG.iter().any(|e| e.tags().contains(tag));
            let dynamic = matches!(*tag, "spire" | "fitness");
            assert!(curated || dynamic, "filter tag {tag} matches nothing");
        }
    }

    #[test]
    fn serials_count_down_from_total() {
        assert_eq!(serial(0), format!("№ {:04}", LOG.len()));
        assert_eq!(serial(LOG.len() - 1), "№ 0001");
    }
}
