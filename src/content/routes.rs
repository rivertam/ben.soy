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
        "/llms".to_string(),
        "/lifting/log".to_string(),
        "/login".to_string(),
    ];
    routes.extend(POSTS.iter().map(|post| format!("/thoughts/{}", post.slug)));
    routes.extend(INTERESTS.iter().map(|i| format!("/{}", i.slug)));
    routes
}
