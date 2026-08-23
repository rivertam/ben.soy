//! The logbook: the site's master feed, newest first. Posts register beside
//! their page; the updates below are the only entries authored here. The
//! `/log` timeline and `/feed.xml` both derive from the merged registry.
//!
//! Serial numbers count from the oldest entry: entry at index `i` is
//! `№ {LOG.len() - i}` (zero-padded to four digits, see [`serial`]).

use std::sync::LazyLock;

use crate::content::posts::{POSTS, PostKind, PostPhoto};

/// One logbook entry. The variants render differently (card / pull-quote /
/// one-liner) but share a date and tags for filtering.
#[derive(Clone, Copy)]
pub enum Entry {
    /// A full post, living at `/thoughts/{slug}`.
    Essay {
        date: &'static str,
        title: &'static str,
        teaser: &'static str,
        slug: &'static str,
        photo: Option<PostPhoto>,
        read_label: &'static str,
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

/// An entry's kind, for the log page's `?kind=` filter.
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

const UPDATES: [Entry; 3] = [
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
    Entry::Update {
        date: "2025-06-02",
        stamp: "win",
        label: "spire",
        body: "Ascension 20, with an annotated run synopsis.",
        href: "/spire",
        link_label: "spire →",
        tags: &["games"],
    },
];

pub static LOG: LazyLock<Vec<Entry>> = LazyLock::new(|| {
    let mut entries = UPDATES.to_vec();
    entries.extend(POSTS.iter().map(|post| match post.kind {
        PostKind::Essay => Entry::Essay {
            date: post.date,
            title: post.title,
            teaser: post.teaser,
            slug: post.slug,
            photo: post.photo,
            read_label: post.read_label,
            tags: post.tags,
        },
        PostKind::Note { body, source } => Entry::Note {
            date: post.date,
            body,
            source,
            slug: post.slug,
            tags: post.tags,
        },
    }));
    entries.sort_unstable_by(|a, b| b.date().cmp(a.date()));
    entries
});

/// The log page's fixed filter-row tag chips. `spire` and `fitness` filter
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
    fn every_post_becomes_exactly_one_log_entry() {
        for post in POSTS.iter() {
            let matches = LOG
                .iter()
                .filter(|entry| match entry {
                    Entry::Essay { slug, .. } | Entry::Note { slug, .. } => *slug == post.slug,
                    Entry::Update { .. } => false,
                })
                .count();
            assert_eq!(matches, 1, "log entries for {}", post.slug);
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
