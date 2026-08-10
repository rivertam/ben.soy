//! Small shared helpers with no better home.

use topcoat::router::{HeaderMap, header};
use url::Url;

/// Percent-encode a URL query value: everything outside the unreserved set
/// (RFC 3986) becomes `%XX`, so arbitrary tag/tech names round-trip.
pub fn urlencode(raw: &str) -> String {
    let mut encoded = String::new();
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Browser writes need positive same-origin evidence. Accepting neither Origin
/// nor Fetch Metadata would turn a cross-site `no-cors` form/fetch into a
/// storage primitive.
pub fn is_same_origin(headers: &HeaderMap) -> bool {
    let fetch_site = header_text(headers, "sec-fetch-site");
    if fetch_site.is_some_and(|value| !matches!(value, "same-origin" | "none")) {
        return false;
    }

    let expected = expected_origin(headers);
    let origin = header_text(headers, header::ORIGIN).and_then(normalized_origin);
    if let Some(origin) = origin {
        return expected.as_deref() == Some(origin.as_str());
    }
    if header_text(headers, header::ORIGIN).is_some() {
        return false;
    }

    let referer = header_text(headers, header::REFERER).and_then(normalized_origin);
    if let Some(referer) = referer {
        return expected.as_deref() == Some(referer.as_str());
    }

    matches!(fetch_site, Some("same-origin" | "none"))
}

fn header_text(
    headers: &HeaderMap,
    name: impl topcoat::router::header::AsHeaderName,
) -> Option<&str> {
    headers.get(name)?.to_str().ok()
}

fn expected_origin(headers: &HeaderMap) -> Option<String> {
    if let Ok(site_origin) = std::env::var("SITE_ORIGIN") {
        return normalized_origin(&site_origin);
    }
    let host = header_text(headers, header::HOST)?;
    let scheme = header_text(headers, "x-forwarded-proto")
        .filter(|value| matches!(*value, "http" | "https"))
        .unwrap_or("http");
    normalized_origin(&format!("{scheme}://{host}"))
}

fn normalized_origin(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;
    use topcoat::router::HeaderValue;

    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("benjisponge.com"));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        headers
    }

    #[test]
    fn same_origin_requires_positive_browser_evidence() {
        assert!(is_same_origin(&headers()));

        let mut cross_site = headers();
        cross_site.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!is_same_origin(&cross_site));

        let mut wrong_origin = headers();
        wrong_origin.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://attacker.example"),
        );
        assert!(!is_same_origin(&wrong_origin));

        let mut no_evidence = HeaderMap::new();
        no_evidence.insert(header::HOST, HeaderValue::from_static("benjisponge.com"));
        assert!(!is_same_origin(&no_evidence));
    }
}
