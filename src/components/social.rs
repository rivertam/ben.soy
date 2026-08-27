//! Search and link-preview metadata for every HTML document.
//!
//! The shell calls this once, after the page route has chosen its title. The
//! registries supply descriptions for posts and interests automatically;
//! data-backed pages can pass a more specific description. All URLs are
//! absolute because Discord, Messages, and other unfurlers fetch the HTML
//! outside a browser and do not reliably resolve relative social images.

use topcoat::{
    Result,
    asset::{Asset, asset, asset_config},
    context::Cx,
    router::request::uri,
    view::{View, view},
};
use url::Url;

use crate::content::{access::hidden_page, interests::INTERESTS, posts::post_for_path};

const SITE_NAME: &str = "Ben Berman";
const DEFAULT_ORIGIN: &str = "https://ben.soy";
const DEFAULT_DESCRIPTION: &str =
    "Ben Berman’s personal logbook: writing, projects, fitness, Felix, and assorted curiosities.";
const DEFAULT_IMAGE_ALT: &str =
    "Ben Berman’s site rendered as an olive-green terminal session with oxide-orange accents.";

/// The one general-purpose landscape image. The editable SVG lives beside
/// it; social crawlers receive this rendered PNG because raster support is
/// substantially more consistent than SVG support in chat clients.
const DEFAULT_SOCIAL_IMAGE: Asset = asset!("./social/share-card.png");

/// A page's title plus the rare social overrides that cannot be inferred from
/// its route registry. Most shell calls pass a `&str` directly; data-backed
/// pages use the builder methods for richer copy or a stateful canonical URL.
pub struct PageMeta {
    title: String,
    description: String,
    canonical_path: String,
}

impl PageMeta {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            canonical_path: String::new(),
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn canonical_path(mut self, canonical_path: impl Into<String>) -> Self {
        self.canonical_path = canonical_path.into();
        self
    }

    pub(super) fn title(&self) -> &str {
        &self.title
    }
}

impl From<&str> for PageMeta {
    fn from(title: &str) -> Self {
        Self::new(title)
    }
}

impl From<String> for PageMeta {
    fn from(title: String) -> Self {
        Self::new(title)
    }
}

/// Fully resolved metadata, ready to become tags. Keeping this representation
/// independent of Topcoat's view makes the exact crawler contract testable.
pub(super) struct SocialMeta {
    pub title: String,
    pub description: String,
    pub canonical_url: String,
    pub image_url: String,
    pub image_type: String,
    pub image_width: u32,
    pub image_height: u32,
    pub image_alt: &'static str,
    pub object_type: &'static str,
    pub published_time: Option<&'static str>,
    pub article_tags: &'static [&'static str],
    pub robots: &'static str,
}

/// Resolve registry defaults, canonical URLs, the best available image, and
/// indexing policy for one shell render.
pub(super) fn metadata(cx: &Cx, page: &PageMeta) -> SocialMeta {
    let request_path = uri(cx).path();
    let post = post_for_path(request_path);
    let origin = site_origin();
    let canonical_path = if page.canonical_path.is_empty() {
        request_path
    } else {
        page.canonical_path.as_str()
    };
    let canonical_url = absolute_site_url(&origin, canonical_path);
    let description = if page.description.trim().is_empty() {
        description_for_path(request_path)
    } else {
        page.description.trim().to_string()
    };

    let (image, image_width, image_height, image_alt) = post.and_then(|post| post.photo).map_or(
        (DEFAULT_SOCIAL_IMAGE, 1200, 630, DEFAULT_IMAGE_ALT),
        |photo| (photo.src, photo.width, photo.height, photo.alt),
    );
    let assets = asset_config(cx);
    let resolved_image = assets.resolve(image);
    let image_url = absolute_asset_url(&origin, &resolved_image);
    let image_type = assets
        .get(image)
        .expect("declared social image is present in the asset bundle")
        .content_type()
        .to_string();

    let title = if page.title.is_empty() {
        SITE_NAME.to_string()
    } else {
        page.title.clone()
    };
    let indexable = is_indexable_path(request_path);

    SocialMeta {
        title,
        description,
        canonical_url,
        image_url,
        image_type,
        image_width,
        image_height,
        image_alt,
        object_type: if post.is_some() { "article" } else { "website" },
        published_time: post.map(|post| post.date),
        article_tags: post.map_or(&[], |post| post.tags),
        robots: if indexable {
            "index,follow,max-image-preview:large"
        } else {
            "noindex,nofollow,noarchive"
        },
    }
}

/// Render the exact Open Graph, Twitter Card, canonical, and ordinary search
/// metadata contract. Values stay in typed view attributes so Topcoat escapes
/// titles and descriptions originating in database-backed pages.
pub(super) async fn head(cx: &Cx, meta: &SocialMeta) -> Result<View> {
    let __cx = cx;
    let image_width = meta.image_width.to_string();
    let image_height = meta.image_height.to_string();
    let secure_image = meta.image_url.starts_with("https://");
    view! {
        <meta name="description" content=(meta.description.as_str())>
        <meta name="author" content=(SITE_NAME)>
        <meta name="robots" content=(meta.robots)>
        <link rel="canonical" href=(meta.canonical_url.as_str())>

        <meta property="og:title" content=(meta.title.as_str())>
        <meta property="og:type" content=(meta.object_type)>
        <meta property="og:url" content=(meta.canonical_url.as_str())>
        <meta property="og:description" content=(meta.description.as_str())>
        <meta property="og:site_name" content=(SITE_NAME)>
        <meta property="og:locale" content="en_US">
        <meta property="og:image" content=(meta.image_url.as_str())>
        if secure_image {
            <meta property="og:image:secure_url" content=(meta.image_url.as_str())>
        }
        <meta property="og:image:type" content=(meta.image_type.as_str())>
        <meta property="og:image:width" content=(image_width.as_str())>
        <meta property="og:image:height" content=(image_height.as_str())>
        <meta property="og:image:alt" content=(meta.image_alt)>

        if let Some(published_time) = meta.published_time {
            <meta property="article:published_time" content=(published_time)>
            <meta property="article:section" content="Thoughts">
            for tag in meta.article_tags {
                <meta property="article:tag" content=(*tag)>
            }
        }

        <meta name="twitter:card" content="summary_large_image">
        <meta name="twitter:title" content=(meta.title.as_str())>
        <meta name="twitter:description" content=(meta.description.as_str())>
        <meta name="twitter:image" content=(meta.image_url.as_str())>
        <meta name="twitter:image:alt" content=(meta.image_alt)>
    }
}

fn site_origin() -> String {
    std::env::var("SITE_ORIGIN")
        .ok()
        .and_then(|value| normalized_origin(&value))
        .unwrap_or_else(|| DEFAULT_ORIGIN.to_string())
}

fn normalized_origin(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.origin().ascii_serialization())
}

fn absolute_site_url(origin: &str, path: &str) -> String {
    if path.starts_with('/') {
        format!("{origin}{path}")
    } else {
        format!("{origin}/{path}")
    }
}

fn absolute_asset_url(origin: &str, resolved: &str) -> String {
    if Url::parse(resolved).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
        resolved.to_string()
    } else {
        absolute_site_url(origin, resolved)
    }
}

/// Human copy for every page family. Posts and interests remain
/// single-sourced in their registries; the small fixed-page map covers the
/// routes that deliberately live outside those registries.
fn description_for_path(path: &str) -> String {
    if let Some(post) = post_for_path(path) {
        return post.teaser.to_string();
    }
    if let Some(hidden) = hidden_page(path) {
        return hidden.teaser.to_string();
    }
    let fixed = match path {
        "/" => DEFAULT_DESCRIPTION,
        "/thoughts" => "Thoughts of varying seriousness and length from Ben Berman.",
        "/resume" => {
            "Ben Berman’s résumé: software engineering, technical leadership, and the projects behind the timeline."
        }
        "/llms" => {
            "How Ben Berman uses LLMs to build and fact-check this site, plus the limits and tradeoffs he sees."
        }
        "/login" => "Sign in to comment or open a private page that Ben Berman shared with you.",
        path if path == "/admin" || path.starts_with("/admin/") => {
            "Private administration for ben.soy."
        }
        path if path == "/diary" || path.starts_with("/diary/") => {
            "Ben Berman’s private, local-first diary."
        }
        "/fitness/share" => {
            "Review a shared fitness link before adding it to Ben Berman’s training log."
        }
        "" => DEFAULT_DESCRIPTION,
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }

    interest_for_path(path).map_or_else(
        || "A page from Ben Berman’s personal logbook.".to_string(),
        |interest| interest.teaser.to_string(),
    )
}

fn interest_for_path(path: &str) -> Option<&'static crate::content::interests::Interest> {
    let root = path.strip_prefix('/')?.split('/').next()?;
    INTERESTS.iter().find(|interest| interest.slug == root)
}

/// Public pages that search engines should index. Private utilities, login,
/// admin, and the catch-all 404 still get useful unfurl metadata but explicitly
/// stay out of search results.
fn is_indexable_path(path: &str) -> bool {
    if post_for_path(path).is_some() {
        return true;
    }
    if matches!(
        path,
        "/" | "/thoughts" | "/resume" | "/llms" | "/fitness/log"
    ) {
        return true;
    }

    if INTERESTS
        .iter()
        .any(|interest| path == format!("/{}", interest.slug))
    {
        return true;
    }

    one_path_segment(path, "/felix/")
        || one_path_segment(path, "/swing/")
        || one_path_segment(path, "/fitness/lift/")
        || one_path_segment(path, "/fitness/exercise/")
        || two_path_segments(path, "/fitness/run/")
}

fn one_path_segment(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
}

fn two_path_segments(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix).is_some_and(|rest| {
        let mut segments = rest.split('/');
        matches!(
            (segments.next(), segments.next(), segments.next()),
            (Some(first), Some(second), None) if !first.is_empty() && !second.is_empty()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{interests::INTERESTS, posts::POSTS, routes::site_routes};
    use topcoat::context::Cx;

    fn fixture() -> SocialMeta {
        SocialMeta {
            title: "How bad are planes?".to_string(),
            description: "Why Ben doesn’t generally fly for leisure & what the numbers say."
                .to_string(),
            canonical_url: "https://ben.soy/thoughts/how-bad-are-planes".to_string(),
            image_url: "https://ben.soy/_topcoat/assets/share-card-deadbeef.png".to_string(),
            image_type: "image/png".to_string(),
            image_width: 1200,
            image_height: 630,
            image_alt: DEFAULT_IMAGE_ALT,
            object_type: "article",
            published_time: Some("2026-07-12"),
            article_tags: &["climate", "planes"],
            robots: "index,follow,max-image-preview:large",
        }
    }

    #[tokio::test]
    async fn head_contains_the_complete_unfurler_contract_and_escapes_copy() {
        let cx = Cx::default();
        let html = head(&cx, &fixture()).await.unwrap().render(&cx);
        for needle in [
            r#"<meta name="description""#,
            r#"<link rel="canonical" href="https://ben.soy/thoughts/how-bad-are-planes">"#,
            r#"<meta property="og:title" content="How bad are planes?">"#,
            r#"<meta property="og:type" content="article">"#,
            r#"<meta property="og:url" content="https://ben.soy/thoughts/how-bad-are-planes">"#,
            r#"<meta property="og:image:type" content="image/png">"#,
            r#"<meta property="og:image:width" content="1200">"#,
            r#"<meta property="og:image:height" content="630">"#,
            r#"<meta property="og:image:alt""#,
            r#"<meta property="article:published_time" content="2026-07-12">"#,
            r#"<meta property="article:tag" content="planes">"#,
            r#"<meta name="twitter:card" content="summary_large_image">"#,
            r#"<meta name="twitter:image:alt""#,
        ] {
            assert!(html.contains(needle), "missing {needle} in {html}");
        }
        assert!(html.contains("leisure &amp; what"));
    }

    #[test]
    fn registries_and_fixed_pages_all_have_descriptions_and_indexing_rules() {
        for route in site_routes() {
            assert!(!description_for_path(&route).trim().is_empty(), "{route}");
            assert_eq!(is_indexable_path(&route), route != "/login", "{route}");
        }
        for post in POSTS.iter() {
            let path = format!("/thoughts/{}", post.slug);
            assert_eq!(description_for_path(&path), post.teaser);
            assert!(is_indexable_path(&path));
        }
        for interest in INTERESTS.iter() {
            let path = format!("/{}", interest.slug);
            assert_eq!(description_for_path(&path), interest.teaser);
            assert!(is_indexable_path(&path));
        }
    }

    #[test]
    fn dynamic_public_routes_are_indexable_but_private_and_unknown_pages_are_not() {
        for path in [
            "/felix/2025-waterfront",
            "/swing/with-eileen",
            "/fitness/lift/2026-07-21T10-39-04-04-00",
            "/fitness/exercise/Full%20Squat",
            "/fitness/run/2026-07-20T19-45-00-04-00/deadbeef",
        ] {
            assert!(is_indexable_path(path), "{path}");
            assert!(!description_for_path(path).is_empty());
        }
        for path in [
            "/login",
            "/admin",
            "/admin/permissions",
            "/diary",
            "/diary/2026-08-23T10-11-12-04-00",
            "/motorcycles",
            "/podrick",
            "/fitness/share",
            "/not-a-page",
            "/fitness/not-a-page",
        ] {
            assert!(!is_indexable_path(path), "{path}");
            assert!(!description_for_path(path).is_empty());
        }
    }

    #[test]
    fn canonical_and_asset_urls_are_absolute_without_double_slashes() {
        assert_eq!(
            absolute_site_url("https://ben.soy", "/resume?tech=Rust"),
            "https://ben.soy/resume?tech=Rust"
        );
        assert_eq!(
            absolute_asset_url("http://localhost:3000", "/_topcoat/assets/card.png"),
            "http://localhost:3000/_topcoat/assets/card.png"
        );
        assert_eq!(
            absolute_asset_url("http://localhost:3000", "https://cdn.example/card.png"),
            "https://cdn.example/card.png"
        );
    }

    #[test]
    fn default_share_card_is_the_declared_large_landscape_png() {
        const PNG: &[u8] = include_bytes!("social/share-card.png");
        assert_eq!(&PNG[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(PNG[16..20].try_into().unwrap()), 1200);
        assert_eq!(u32::from_be_bytes(PNG[20..24].try_into().unwrap()), 630);
        assert!(PNG.len() < 5 * 1024 * 1024);
    }
}
