//! Distributed post registry. Each page owns its metadata beside its route;
//! inventory gathers those declarations here so indexes and feeds cannot
//! drift from the page copy.

use std::sync::LazyLock;

use topcoat::asset::Asset;

#[derive(Clone, Copy)]
pub enum PostKind {
    Essay,
    Note {
        body: &'static str,
        source: &'static str,
    },
}

#[derive(Clone, Copy)]
pub struct PostPhoto {
    pub src: Asset,
    pub alt: &'static str,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy)]
pub struct Post {
    pub slug: &'static str,
    pub title: &'static str,
    pub date: &'static str,
    pub teaser: &'static str,
    pub photo: Option<PostPhoto>,
    pub read_label: &'static str,
    pub tags: &'static [&'static str],
    pub kind: PostKind,
}

inventory::collect!(Post);

/// Declare a post beside its page. The generated `POST` constant is also
/// available to the page itself, keeping its title/date/copy single-sourced.
#[macro_export]
macro_rules! register_post {
    (@photo) => {
        None
    };
    (@photo $src:expr, $alt:literal, $width:literal, $height:literal) => {
        Some($crate::content::posts::PostPhoto {
            src: $src,
            alt: $alt,
            width: $width,
            height: $height,
        })
    };
    (@read_label) => {
        "read →"
    };
    (@read_label $label:literal) => {
        $label
    };
    (
        essay,
        slug: $slug:literal,
        title: $title:literal,
        date: $date:literal,
        teaser: $teaser:literal,
        $(photo: {
            src: $photo_src:expr,
            alt: $photo_alt:literal,
            width: $photo_width:literal,
            height: $photo_height:literal $(,)?
        },)?
        $(read_label: $read_label:literal,)?
        tags: $tags:expr $(,)?
    ) => {
        const POST: $crate::content::posts::Post = $crate::content::posts::Post {
            slug: $slug,
            title: $title,
            date: $date,
            teaser: $teaser,
            photo: $crate::register_post!(
                @photo $($photo_src, $photo_alt, $photo_width, $photo_height)?
            ),
            read_label: $crate::register_post!(@read_label $($read_label)?),
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
        $(photo: {
            src: $photo_src:expr,
            alt: $photo_alt:literal,
            width: $photo_width:literal,
            height: $photo_height:literal $(,)?
        },)?
        $(read_label: $read_label:literal,)?
        tags: $tags:expr $(,)?
    ) => {
        const POST: $crate::content::posts::Post = $crate::content::posts::Post {
            slug: $slug,
            title: $title,
            date: $date,
            teaser: $teaser,
            photo: $crate::register_post!(
                @photo $($photo_src, $photo_alt, $photo_width, $photo_height)?
            ),
            read_label: $crate::register_post!(@read_label $($read_label)?),
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
                    && !post.read_label.is_empty()
                    && !post.tags.is_empty()
            );
            assert!(post.tags.iter().all(|tag| {
                !tag.is_empty() && tag.chars().all(|character| character.is_ascii_lowercase())
            }));
            if let Some(photo) = post.photo {
                assert!(!photo.alt.is_empty());
                assert!(photo.width > 0 && photo.height > 0);
            }
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
