//! Distributed post registry. Each page owns its metadata beside its route;
//! inventory gathers those declarations here so indexes and feeds cannot
//! drift from the page copy.

use std::sync::LazyLock;

use topcoat::{
    asset::Asset,
    context::Cx,
    router::{Body, IntoResponse, RouteFuture, error::redirect_permanent, uri},
};

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
    pub shortlink: Option<&'static str>,
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
/// An optional `shortlink: "name"` after `slug` registers a permanent redirect
/// from `/name` to the post's canonical `/thoughts/{slug}` path.
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
    (@shortlink) => {
        None
    };
    (@shortlink $shortlink:literal) => {
        Some($shortlink)
    };
    (@shortlink_route) => {};
    (@shortlink_route $shortlink:literal) => {
        const __POST_SHORTLINK: ::topcoat::router::RouteFn =
            ::topcoat::router::RouteFn::const_new(
                ::topcoat::router::OwnedMethods::One(::topcoat::router::Method::GET),
                ::std::borrow::Cow::Borrowed(::topcoat::router::Path::new(concat!(
                    "/",
                    $shortlink
                ))),
                $crate::content::posts::shortlink_handler,
            );
        inventory::submit! { __POST_SHORTLINK }
    };
    (
        essay,
        slug: $slug:literal,
        $(shortlink: $shortlink:literal,)?
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
            shortlink: $crate::register_post!(@shortlink $($shortlink)?),
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
        $crate::register_post!(@shortlink_route $($shortlink)?);
    };
    (
        note,
        slug: $slug:literal,
        $(shortlink: $shortlink:literal,)?
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
            shortlink: $crate::register_post!(@shortlink $($shortlink)?),
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
        $crate::register_post!(@shortlink_route $($shortlink)?);
    };
}

/// All discovered posts, newest first. Inventory iteration order is
/// unspecified, so every consumer shares this explicitly sorted view.
pub static POSTS: LazyLock<Vec<&'static Post>> = LazyLock::new(|| {
    let mut posts: Vec<_> = inventory::iter::<Post>.into_iter().collect();
    posts.sort_unstable_by(|a, b| b.date.cmp(a.date).then_with(|| a.slug.cmp(b.slug)));
    posts
});

/// The registered post at an exact canonical `/thoughts/{slug}` path.
///
/// Matching through the registry keeps page-wide behavior attached to posts,
/// not merely to anything that happens to live below the `/thoughts` prefix.
pub fn post_for_path(path: &str) -> Option<&'static Post> {
    let slug = path.strip_prefix("/thoughts/")?;
    POSTS.iter().copied().find(|post| post.slug == slug)
}

/// The registered post for an exact root-level shortlink path.
pub fn post_for_shortlink_path(path: &str) -> Option<&'static Post> {
    let shortlink = path.strip_prefix('/')?;
    if shortlink.is_empty() || shortlink.contains('/') {
        return None;
    }
    POSTS
        .iter()
        .copied()
        .find(|post| post.shortlink == Some(shortlink))
}

#[doc(hidden)]
pub(crate) fn shortlink_handler<'cx>(cx: &'cx Cx, _body: Body) -> RouteFuture<'cx> {
    Box::pin(async move {
        let request_uri = uri(cx);
        let post = post_for_shortlink_path(request_uri.path())
            .expect("register_post! only routes registered shortlinks");
        let mut target = format!("/thoughts/{}", post.slug);
        if let Some(query) = request_uri.query() {
            target.push('?');
            target.push_str(query);
        }
        redirect_permanent(&target).into_response(cx)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_slug(slug: &str) -> bool {
        !slug.is_empty()
            && slug.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            })
    }

    #[test]
    fn metadata_is_unique_complete_and_newest_first() {
        assert!(!POSTS.is_empty());
        for post in POSTS.iter() {
            assert!(
                canonical_slug(post.slug),
                "non-canonical slug {}",
                post.slug
            );
            if let Some(shortlink) = post.shortlink {
                assert!(
                    canonical_slug(shortlink),
                    "non-canonical shortlink {shortlink}"
                );
            }
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
            if let Some(shortlink) = post.shortlink {
                assert!(
                    POSTS[index + 1..]
                        .iter()
                        .all(|other| other.shortlink != Some(shortlink)),
                    "duplicate post shortlink {shortlink}"
                );
            }
        }
    }

    #[test]
    fn exact_thought_paths_resolve_through_the_registry() {
        for post in POSTS.iter() {
            let path = format!("/thoughts/{}", post.slug);
            assert_eq!(
                post_for_path(&path).map(|found| found.slug),
                Some(post.slug)
            );
            assert!(post_for_path(&format!("{path}/")).is_none());
            assert!(post_for_path(&format!("{path}/replies")).is_none());
        }

        assert!(post_for_path("/thoughts").is_none());
        assert!(post_for_path("/thoughts/").is_none());
        assert!(post_for_path("/log").is_none());
    }

    #[test]
    fn exact_shortlink_paths_resolve_through_the_registry() {
        for post in POSTS.iter() {
            let Some(shortlink) = post.shortlink else {
                continue;
            };
            let path = format!("/{shortlink}");
            assert_eq!(
                post_for_shortlink_path(&path).map(|found| found.slug),
                Some(post.slug)
            );
            assert!(post_for_shortlink_path(&format!("{path}/")).is_none());
            assert!(post_for_shortlink_path(&format!("{path}/more")).is_none());
        }

        assert!(post_for_shortlink_path("/").is_none());
        assert!(post_for_shortlink_path("/not-a-shortlink").is_none());
        assert!(post_for_shortlink_path("thoughts").is_none());
    }

    #[tokio::test]
    async fn shortlinks_redirect_permanently_and_preserve_the_query() {
        use topcoat::router::{Body, Request, Router, RouterBuilderDiscoverExt, header};

        let router = Router::builder().discover().build();
        for (source, target) in [
            (
                "/crops?food=tofu&meal=2",
                "/thoughts/crop-deaths?food=tofu&meal=2",
            ),
            (
                "/simulation?from=old-link",
                "/thoughts/simulation?from=old-link",
            ),
            ("/puzzles?from=old-link", "/thoughts/puzzles?from=old-link"),
        ] {
            let request = Request::builder().uri(source).body(Body::empty()).unwrap();
            let response = router.handle(request).await;

            assert_eq!(response.status(), 308, "{source}");
            assert_eq!(
                response.headers().get(header::LOCATION).unwrap(),
                target,
                "{source}"
            );
        }
    }
}
