//! The site's canonical route list, derived from the page registries. The
//! 404's suggestion index reads this, so a page that exists here but not in
//! the router — or vice versa — is a bug in exactly one place.

use crate::content::{interests::INTERESTS, posts::POSTS};

/// Every real route on the site, fixed pages first.
pub fn site_routes() -> Vec<String> {
    let mut routes = vec![
        "/".to_string(),
        "/thoughts".to_string(),
        "/resume".to_string(),
        "/interests".to_string(),
        "/analytics".to_string(),
        "/llms".to_string(),
        "/lifting/log".to_string(),
        "/login".to_string(),
    ];
    routes.extend(POSTS.iter().map(|post| format!("/thoughts/{}", post.slug)));
    routes.extend(INTERESTS.iter().map(|i| format!("/{}", i.slug)));
    routes
}

/// Whether a canonical browser path belongs to a page that can report
/// analytics.
///
/// Gallery and workout details have one dynamic segment and therefore cannot
/// all live in the fixed route registry. The 404 page does not load the
/// tracker, so accepting their bounded shapes does not turn ordinary missing
/// URLs into public analytics entries.
pub fn is_trackable_route(path: &str) -> bool {
    if site_routes().iter().any(|route| route == path) {
        return true;
    }

    ["/felix/", "/swing/", "/lifting/"].iter().any(|prefix| {
        path.strip_prefix(prefix).is_some_and(|segment| {
            !segment.is_empty()
                && segment.len() <= 220
                && !segment.contains('/')
                && !segment.chars().any(char::is_control)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_routes_include_dynamic_details_but_not_arbitrary_404s() {
        assert!(is_trackable_route("/resume"));
        assert!(is_trackable_route("/felix/2025-rug"));
        assert!(is_trackable_route("/swing/with-eileen"));
        assert!(is_trackable_route("/lifting/2026-07-18T16-19-36-04-00"));
        assert!(!is_trackable_route("/private-canary/alice"));
        assert!(!is_trackable_route("/felix/one/two"));
    }
}
