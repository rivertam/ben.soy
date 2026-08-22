//! The site-wide response layer. Topcoat's discovery allows exactly one
//! `#[layer]` per path — a second `#[layer("/")]` panics at router build —
//! so every whole-site response behavior lives in this one fn: add here,
//! never as a sibling layer.
//!
//! Current behaviors, in order:
//!
//! 1. **Viewer privacy.** Any request bearing the viewer cookie gets
//!    `Cache-Control: private, no-store`, whatever the page declared.
//!    Signed-in HTML is personalized — the shell's nav and footer render the
//!    viewer's grants and email — while pages keep declaring the cache
//!    headers their anonymous variants want (/log says `s-maxage=60`, the
//!    shell default `s-maxage=86400`). First-mention-wins can't help: pages
//!    emit their header before `shell()` runs, so without this chokepoint a
//!    signed-in render would land in the Cloudflare cache under the page's
//!    public TTL and serve that viewer's page (and email) to everyone.
//!    Keying on cookie PRESENCE, not validity: a garbage `__Host-viewer`
//!    curl'd at the site must fail closed to uncacheable, and deciding must
//!    not require the encrypted jar. The one exemption is by RESPONSE — the
//!    `immutable` hashed assets, shared bytes worth caching in signed-in
//!    browsers. Never exempt by request path: `/_topcoat/junk` falls through
//!    to the catch-all 404, which renders the personalized shell under its
//!    public default TTL (a review caught exactly that hole).
//! 2. **Native-share privacy.** Every `/fitness/share` response (and its
//!    short-lived `/running/share` compatibility alias), including a
//!    signed-out framework redirect or malformed-query error produced before
//!    the page can attach headers, is `no-store` with no referrer.
//! 3. **Em-dash links.** Em dashes in HTML page bodies become `/llms` links
//!    (`crate::emdash`).

use topcoat::{
    Result,
    context::CxBuilder,
    router::{Body, HeaderMap, HeaderValue, IntoResponse, Next, Response, header, layer, to_bytes},
};

use super::login::VIEWER_COOKIE_BROWSER_NAME;
use crate::emdash::link_em_dashes;

#[layer("/")]
async fn site_responses(cx: &mut CxBuilder, body: Body, next: Next<'_>) -> Result<Response> {
    let (personalized, native_share) = cx
        .get::<http::request::Parts>()
        .map(|parts| {
            (
                names_viewer_cookie(&parts.headers),
                is_native_share_path(parts.uri.path()),
            )
        })
        .unwrap_or((false, false));
    let (mut response, from_error) = match next.run(cx, body).await {
        Ok(response) => (response, false),
        // Framework error responses (404/405 terminals, query-param
        // redirects, handler 500s) normally render OUTSIDE every layer and
        // would escape the stamp below; convert personalized ones here.
        Err(error) if personalized || native_share => {
            (IntoResponse::into_response(error, cx)?, true)
        }
        Err(error) => return Err(error),
    };
    if personalized && !immutable(response.headers().get(header::CACHE_CONTROL)) {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        );
    } else if native_share {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if native_share {
        response.headers_mut().insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
    }

    // Error responses never met the em-dash rewrite when the framework
    // rendered them, so converted ones skip it too.
    if from_error || !is_html(response.headers().get(header::CONTENT_TYPE)) {
        return Ok(response);
    }
    let (mut parts, body) = response.into_parts();
    let bytes = to_bytes(body, usize::MAX).await.map_err(|err| {
        std::io::Error::other(format!("response layer: failed to read body: {err}"))
    })?;
    let html = String::from_utf8_lossy(&bytes);
    let rewritten = link_em_dashes(&html);
    parts.headers.remove(header::CONTENT_LENGTH);
    Ok(Response::from_parts(parts, Body::from(rewritten)))
}

fn is_native_share_path(path: &str) -> bool {
    matches!(path, "/fitness/share" | "/running/share")
}

/// Whether any `Cookie` header names the viewer cookie. Cookie syntax is
/// `name=value; name2=value2`, possibly split across headers (HTTP/2 does).
/// Name-exact on purpose: the name appearing inside another cookie's name or
/// value must not force every stranger's response uncacheable.
fn names_viewer_cookie(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .any(|(name, _)| name.trim() == VIEWER_COOKIE_BROWSER_NAME)
}

/// Whether the response already declared itself `immutable` — true only of
/// the hashed `/_topcoat/` assets, whose bytes are shared and never
/// personalized. Everything else a signed-in request produces, including any
/// 404 fallback, must not outlive the request in a cache.
fn immutable(cache_control: Option<&HeaderValue>) -> bool {
    cache_control
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("immutable"))
}

fn is_html(content_type: Option<&header::HeaderValue>) -> bool {
    match content_type {
        None => true,
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|s| s.split(';').next())
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in values {
            map.append(header::COOKIE, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn finds_the_viewer_cookie_by_exact_name() {
        assert!(names_viewer_cookie(&headers(&["__Host-viewer=abc"])));
        assert!(names_viewer_cookie(&headers(&[
            "theme=dark; __Host-viewer=abc; other=1"
        ])));
        // HTTP/2 may split the cookie list across several headers.
        assert!(names_viewer_cookie(&headers(&[
            "theme=dark",
            "__Host-viewer=abc"
        ])));
    }

    #[test]
    fn only_immutable_responses_escape_the_override() {
        let value = |s| HeaderValue::from_static(s);
        assert!(immutable(Some(&value(
            "public, max-age=31536000, immutable"
        ))));
        assert!(!immutable(Some(&value(
            "public, max-age=0, s-maxage=86400"
        ))));
        assert!(!immutable(Some(&value("no-store"))));
        assert!(!immutable(None));
    }

    #[test]
    fn ignores_lookalikes_and_values() {
        assert!(!names_viewer_cookie(&headers(&[])));
        assert!(!names_viewer_cookie(&headers(&["theme=dark"])));
        assert!(!names_viewer_cookie(&headers(&["x__Host-viewer=1"])));
        assert!(!names_viewer_cookie(&headers(&["__Host-viewer2=1"])));
        assert!(!names_viewer_cookie(&headers(&["ref=__Host-viewer"])));
        assert!(!names_viewer_cookie(&headers(&["__Host-viewer"])));
    }

    #[test]
    fn only_the_exact_native_share_landing_is_sensitive() {
        assert!(is_native_share_path("/fitness/share"));
        assert!(is_native_share_path("/running/share"));
        assert!(!is_native_share_path("/fitness"));
        assert!(!is_native_share_path("/fitness/share/extra"));
        assert!(!is_native_share_path("/lifting/share"));
    }
}
