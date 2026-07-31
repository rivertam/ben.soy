//! Distributed post registry. Each page owns its metadata beside its route;
//! inventory gathers those declarations here so indexes and feeds cannot
//! drift from the page copy.

use std::sync::LazyLock;

#[derive(Clone, Copy)]
pub enum PostKind {
    Essay,
    Note {
        body: &'static str,
        source: &'static str,
    },
}

#[derive(Clone, Copy)]
pub struct Post {
    pub slug: &'static str,
    pub title: &'static str,
    pub date: &'static str,
    pub teaser: &'static str,
    pub tags: &'static [&'static str],
    pub kind: PostKind,
}

inventory::collect!(Post);

/// Declare a post beside its page. The generated `POST` constant is also
/// available to the page itself, keeping its title/date/copy single-sourced.
#[macro_export]
macro_rules! register_post {
    (
        essay,
        slug: $slug:literal,
        title: $title:literal,
        date: $date:literal,
        teaser: $teaser:literal,
        tags: $tags:expr $(,)?
    ) => {
        const POST: $crate::content::posts::Post = $crate::content::posts::Post {
            slug: $slug,
            title: $title,
            date: $date,
            teaser: $teaser,
            tags: $tags,
            kind: $crate::content::posts::PostKind::Essay,
        };
        inventory::submit! { POST }
    };
    (
        note,
        slug: $slug:literal,
        title: $title:literal,
        date: $date:literal,
        teaser: $teaser:literal,
        source: $source:literal,
        body: $body:literal,
        tags: $tags:expr $(,)?
    ) => {
        const POST: $crate::content::posts::Post = $crate::content::posts::Post {
            slug: $slug,
            title: $title,
            date: $date,
            teaser: $teaser,
            tags: $tags,
            kind: $crate::content::posts::PostKind::Note {
                body: $body,
                source: $source,
            },
        };
        inventory::submit! { POST }
    };
}

/// All discovered posts, newest first. Inventory iteration order is
/// unspecified, so every consumer shares this explicitly sorted view.
pub static POSTS: LazyLock<Vec<&'static Post>> = LazyLock::new(|| {
    let mut posts: Vec<_> = inventory::iter::<Post>.into_iter().collect();
    posts.sort_unstable_by(|a, b| b.date.cmp(a.date).then_with(|| a.slug.cmp(b.slug)));
    posts
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_unique_complete_and_newest_first() {
        assert!(!POSTS.is_empty());
        for post in POSTS.iter() {
            assert!(
                !post.slug.is_empty()
                    && !post.title.is_empty()
                    && !post.date.is_empty()
                    && !post.teaser.is_empty()
                    && !post.tags.is_empty()
            );
            assert!(post.tags.iter().all(|tag| {
                !tag.is_empty() && tag.chars().all(|character| character.is_ascii_lowercase())
            }));
        }
        for pair in POSTS.windows(2) {
            assert!(pair[0].date >= pair[1].date);
        }
        for (index, post) in POSTS.iter().enumerate() {
            assert!(
                POSTS[index + 1..]
                    .iter()
                    .all(|other| other.slug != post.slug),
                "duplicate post slug {}",
                post.slug
            );
        }
    }
}
