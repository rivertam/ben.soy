//! Validation and privacy-preserving enrichment for browser events.

use jiff::{Timestamp, tz::TimeZone};
use serde::Deserialize;
use topcoat::router::{HeaderMap, header};
use url::Url;
use uuid::{Uuid, Variant, Version};

use crate::content::routes::is_trackable_route;

const MAX_PATH_BYTES: usize = 300;
const MAX_LABEL_CHARS: usize = 80;
const MAX_CURRENT_PAGE_BYTES: usize = 4 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventPayload {
    pub version: i64,
    pub id: String,
    pub bootstrap_id: String,
    pub kind: String,
    pub referrer: Option<String>,
    pub timezone: Option<String>,
    pub viewport_width: Option<i64>,
    pub engagement_ms: Option<i64>,
    pub scroll_percent: Option<i64>,
    pub lcp_ms: Option<i64>,
    pub cls_milli: Option<i64>,
    pub navigation_ms: Option<i64>,
    pub target: Option<String>,
}

#[derive(Debug)]
pub struct ValidatedEvent {
    pub id: String,
    pub bootstrap_id: String,
    pub kind: String,
    pub path: String,
    pub referrer_kind: String,
    pub referrer_host: Option<String>,
    pub referrer_path: Option<String>,
    pub country_code: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub device_kind: String,
    pub browser: String,
    pub operating_system: String,
    pub viewport_kind: String,
    pub navigation_kind: Option<String>,
    pub local_hour: Option<i64>,
    pub local_weekday: Option<i64>,
    pub engagement_seconds: Option<i64>,
    pub scroll_percent: Option<i64>,
    pub lcp_milliseconds: Option<i64>,
    pub cls_thousandths: Option<i64>,
    pub navigation_milliseconds: Option<i64>,
    pub target_host: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
}

struct CurrentPage {
    path: String,
    utm_source: Option<String>,
    utm_medium: Option<String>,
    utm_campaign: Option<String>,
}

#[derive(Default)]
struct LocalClock {
    timezone: Option<String>,
    hour: Option<i64>,
    weekday: Option<i64>,
}

impl EventPayload {
    pub fn validate(
        self,
        headers: &HeaderMap,
        occurred_at: i64,
    ) -> Result<ValidatedEvent, &'static str> {
        if self.version != 2 {
            return Err("unsupported version");
        }
        validate_uuid(&self.id).ok_or("bad event id")?;
        validate_uuid(&self.bootstrap_id).ok_or("bad bootstrap id")?;

        if !matches!(self.kind.as_str(), "pageview" | "engagement" | "outbound") {
            return Err("bad event kind");
        }
        let current_page = current_page(headers).ok_or("bad current page")?;
        let request_host = expected_host(headers);
        let pageview = self.kind == "pageview";
        let engagement = self.kind == "engagement";
        let outbound = self.kind == "outbound";

        if pageview {
            if self.target.is_some()
                || self.engagement_ms.is_some()
                || self.scroll_percent.is_some()
                || self.lcp_ms.is_some()
                || self.cls_milli.is_some()
                || self.navigation_ms.is_some()
            {
                return Err("pageview has event-only fields");
            }
        } else if engagement {
            if self.referrer.is_some()
                || self.timezone.is_some()
                || self.viewport_width.is_some()
                || self.target.is_some()
            {
                return Err("engagement has page-only fields");
            }
        } else if self.referrer.is_some()
            || self.timezone.is_some()
            || self.viewport_width.is_some()
            || self.engagement_ms.is_some()
            || self.scroll_percent.is_some()
            || self.lcp_ms.is_some()
            || self.cls_milli.is_some()
            || self.navigation_ms.is_some()
        {
            return Err("outbound has unrelated fields");
        }

        let referrer = if pageview {
            classify_referrer(self.referrer.as_deref(), request_host.as_deref())?
        } else {
            direct_referrer()
        };
        let target_host = if outbound {
            Some(external_origin_host(
                self.target
                    .as_deref()
                    .ok_or("outbound event needs a target")?,
                request_host.as_deref(),
            )?)
        } else {
            None
        };
        let local_clock = if pageview {
            local_clock(self.timezone.as_deref(), occurred_at)?
        } else {
            LocalClock::default()
        };
        let viewport_width = if pageview {
            bounded_optional(self.viewport_width, 1, 10_000, "bad viewport")?
        } else {
            None
        };
        let engagement_seconds = if engagement {
            let milliseconds =
                bounded_required(self.engagement_ms, 0, 7_200_000, "bad engagement")?;
            Some((milliseconds + 500) / 1_000)
        } else {
            None
        };
        let scroll_percent = if engagement {
            bounded_optional(self.scroll_percent, 0, 100, "bad scroll")?
        } else {
            None
        };
        let lcp_milliseconds = if engagement {
            bounded_optional(self.lcp_ms, 0, 120_000, "bad lcp")?
        } else {
            None
        };
        let cls_thousandths = if engagement {
            bounded_optional(self.cls_milli, 0, 100_000, "bad cls")?
        } else {
            None
        };
        let navigation_milliseconds = if engagement {
            bounded_optional(self.navigation_ms, 0, 120_000, "bad navigation timing")?
        } else {
            None
        };

        let user_agent = header_text(headers, header::USER_AGENT).unwrap_or("");
        let hints_mobile = header_text(headers, "sec-ch-ua-mobile") == Some("?1");
        let device_kind = device_kind(user_agent, hints_mobile).to_string();
        let browser = browser_family(user_agent).to_string();
        let operating_system = operating_system(user_agent).to_string();
        let country_code = header_text(headers, "cf-ipcountry").and_then(clean_country);

        Ok(ValidatedEvent {
            id: self.id.to_ascii_lowercase(),
            bootstrap_id: self.bootstrap_id.to_ascii_lowercase(),
            kind: self.kind,
            path: current_page.path,
            referrer_kind: referrer.kind.to_string(),
            referrer_host: referrer.host,
            referrer_path: referrer.path,
            country_code,
            timezone: local_clock.timezone,
            language: preferred_language(headers),
            device_kind,
            browser,
            operating_system,
            viewport_kind: viewport_kind(viewport_width).to_string(),
            navigation_kind: None,
            local_hour: local_clock.hour,
            local_weekday: local_clock.weekday,
            engagement_seconds,
            scroll_percent,
            lcp_milliseconds,
            cls_thousandths,
            navigation_milliseconds,
            target_host,
            utm_source: pageview.then_some(current_page.utm_source).flatten(),
            utm_medium: pageview.then_some(current_page.utm_medium).flatten(),
            utm_campaign: pageview.then_some(current_page.utm_campaign).flatten(),
        })
    }
}

fn validate_uuid(value: &str) -> Option<Uuid> {
    let uuid = Uuid::parse_str(value).ok()?;
    (uuid.get_version() == Some(Version::Random)
        && uuid.get_variant() == Variant::RFC4122
        && uuid.hyphenated().to_string().eq_ignore_ascii_case(value))
    .then_some(uuid)
}

pub(super) fn is_canonical_v4_uuid(value: &str) -> bool {
    validate_uuid(value).is_some()
}

fn bounded_required(
    value: Option<i64>,
    min: i64,
    max: i64,
    error: &'static str,
) -> Result<i64, &'static str> {
    value
        .filter(|value| (min..=max).contains(value))
        .ok_or(error)
}

fn bounded_optional(
    value: Option<i64>,
    min: i64,
    max: i64,
    error: &'static str,
) -> Result<Option<i64>, &'static str> {
    match value {
        Some(value) if (min..=max).contains(&value) => Ok(Some(value)),
        Some(_) => Err(error),
        None => Ok(None),
    }
}

fn clean_path(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > MAX_PATH_BYTES
        || !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || value.contains(['?', '#'])
        || value.chars().any(char::is_control)
        || value.starts_with("/api/")
        || value.starts_with("/_topcoat/")
        || !is_trackable_route(value)
    {
        return None;
    }
    Some(value.to_string())
}

fn clean_optional_label(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty()
        || value.chars().count() > MAX_LABEL_CHARS
        || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value.to_string())
}

fn clean_ascii_label(value: &str, max: usize, extra: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || extra.as_bytes().contains(&byte)))
    .then(|| value.to_string())
}

fn clean_country(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_uppercase();
    (value.len() == 2
        && value.bytes().all(|byte| byte.is_ascii_uppercase())
        && !matches!(value.as_str(), "XX" | "T1"))
    .then_some(value)
}

fn header_text(
    headers: &HeaderMap,
    name: impl topcoat::router::header::AsHeaderName,
) -> Option<&str> {
    headers.get(name)?.to_str().ok()
}

fn viewport_kind(width: Option<i64>) -> &'static str {
    match width {
        Some(0..=639) => "phone",
        Some(640..=1023) => "tablet",
        Some(1024..=1599) => "desktop",
        Some(1600..=10_000) => "wide",
        _ => "unknown",
    }
}

fn device_kind(user_agent: &str, hints_mobile: bool) -> &'static str {
    if user_agent.contains("iPad") || user_agent.contains("Tablet") {
        "tablet"
    } else if hints_mobile
        || user_agent.contains("Mobile")
        || user_agent.contains("iPhone")
        || user_agent.contains("Android")
    {
        "mobile"
    } else if user_agent.is_empty() {
        "unknown"
    } else {
        "desktop"
    }
}

fn browser_family(user_agent: &str) -> &'static str {
    if user_agent.contains("Edg/") {
        "Edge"
    } else if user_agent.contains("Firefox/") || user_agent.contains("FxiOS/") {
        "Firefox"
    } else if user_agent.contains("OPR/") || user_agent.contains("Opera/") {
        "Opera"
    } else if user_agent.contains("Chrome/") || user_agent.contains("CriOS/") {
        "Chrome"
    } else if user_agent.contains("Safari/") {
        "Safari"
    } else {
        "Other"
    }
}

fn operating_system(user_agent: &str) -> &'static str {
    if user_agent.contains("iPhone") || user_agent.contains("iPad") {
        "iOS"
    } else if user_agent.contains("Android") {
        "Android"
    } else if user_agent.contains("Windows") {
        "Windows"
    } else if user_agent.contains("Mac OS X") || user_agent.contains("Macintosh") {
        "macOS"
    } else if user_agent.contains("Linux") {
        "Linux"
    } else {
        "Other"
    }
}

struct Referrer {
    kind: &'static str,
    host: Option<String>,
    path: Option<String>,
}

fn direct_referrer() -> Referrer {
    Referrer {
        kind: "direct",
        host: None,
        path: None,
    }
}

fn classify_referrer(
    value: Option<&str>,
    own_host: Option<&str>,
) -> Result<Referrer, &'static str> {
    let Some(value) = value else {
        return Ok(direct_referrer());
    };
    let url = http_url(value).ok_or("bad referrer")?;
    if url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("bad referrer");
    }
    let host = normalized_host(&url).ok_or("bad referrer")?;
    if own_host == Some(host.as_str()) {
        // A same-host referrer that isn't a trackable route (a hidden page,
        // a 404) must never be stored by name — but the destination pageview
        // is still real data, so degrade to "direct" instead of rejecting
        // the whole event.
        return Ok(match clean_path(url.path()) {
            Some(path) => Referrer {
                kind: "internal",
                host: None,
                path: Some(path),
            },
            None => direct_referrer(),
        });
    };
    if url.path() != "/" {
        return Err("external referrer must be an origin");
    }

    let kind = if host.starts_with("google.")
        || host.starts_with("search.yahoo.")
        || domain_matches(
            &host,
            &[
                "google.com",
                "bing.com",
                "duckduckgo.com",
                "search.brave.com",
                "search.yahoo.com",
                "ecosia.org",
                "kagi.com",
            ],
        ) {
        "search"
    } else if domain_matches(
        &host,
        &[
            "reddit.com",
            "linkedin.com",
            "x.com",
            "twitter.com",
            "facebook.com",
            "instagram.com",
            "bsky.app",
            "news.ycombinator.com",
        ],
    ) {
        "social"
    } else if domain_matches(
        &host,
        &[
            "chatgpt.com",
            "claude.ai",
            "perplexity.ai",
            "copilot.microsoft.com",
        ],
    ) {
        "ai"
    } else {
        "referral"
    };
    Ok(Referrer {
        kind,
        host: Some(host),
        path: None,
    })
}

fn external_origin_host(value: &str, own_host: Option<&str>) -> Result<String, &'static str> {
    let url = http_url(value).ok_or("bad target")?;
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("target must be an origin");
    }
    let host = normalized_host(&url).ok_or("bad target")?;
    if own_host == Some(host.as_str()) {
        return Err("outbound target must be external");
    }
    Ok(host)
}

fn local_clock(value: Option<&str>, occurred_at: i64) -> Result<LocalClock, &'static str> {
    let Some(value) = value else {
        return Ok(LocalClock::default());
    };
    let timezone = clean_ascii_label(value, 64, "_/+-").ok_or("bad timezone")?;
    let zone = TimeZone::get(&timezone).map_err(|_| "bad timezone")?;
    let instant = Timestamp::from_second(occurred_at).map_err(|_| "bad event time")?;
    let local = instant.to_zoned(zone);
    Ok(LocalClock {
        timezone: Some(timezone),
        hour: Some(i64::from(local.hour())),
        weekday: Some(i64::from(local.weekday().to_sunday_zero_offset())),
    })
}

fn http_url(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

fn normalized_host(url: &Url) -> Option<String> {
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

fn domain_matches(host: &str, domains: &[&str]) -> bool {
    domains
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn expected_host(headers: &HeaderMap) -> Option<String> {
    if let Ok(site_origin) = std::env::var("SITE_ORIGIN")
        && let Some(url) = http_url(&site_origin)
    {
        return normalized_host(&url);
    }
    let host = header_text(headers, header::HOST)?
        .split(':')
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

/// Passive analytics honors both browser privacy signals before a cookie,
/// request guard, body parser, or database connection is touched.
pub fn tracking_opted_out(headers: &HeaderMap) -> bool {
    let gpc = headers
        .get_all("sec-gpc")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.trim() == "1");
    let dnt = headers
        .get_all("dnt")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.trim_start().starts_with('1'));
    gpc || dnt
}

/// Browser writes need positive same-origin evidence. This endpoint has no
/// CORS response, and accepting neither Origin nor Fetch Metadata would turn a
/// cross-site `no-cors` form/fetch into a storage primitive.
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

/// The HTTP Referer is the current page, distinct from the acquisition
/// referrer reported by `document.referrer`. It authoritatively supplies the
/// canonical path and the only query parameters analytics retains.
fn current_page(headers: &HeaderMap) -> Option<CurrentPage> {
    let mut values = headers.get_all(header::REFERER).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() || value.len() > MAX_CURRENT_PAGE_BYTES {
        return None;
    }
    let url = http_url(value)?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return None;
    }
    if normalized_origin(url.as_str()).as_deref() != expected_origin(headers).as_deref() {
        return None;
    }
    let path = clean_path(url.path())?;
    Some(CurrentPage {
        path,
        utm_source: campaign_value(&url, "utm_source"),
        utm_medium: campaign_value(&url, "utm_medium"),
        utm_campaign: campaign_value(&url, "utm_campaign"),
    })
}

fn campaign_value(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .and_then(|(_, value)| clean_optional_label(Some(value.as_ref())))
}

pub fn current_page_matches(headers: &HeaderMap, reported_path: &str) -> bool {
    current_page(headers).is_some_and(|page| page.path == reported_path)
}

fn preferred_language(headers: &HeaderMap) -> Option<String> {
    let raw = header_text(headers, header::ACCEPT_LANGUAGE)?;
    if raw.len() > 1_024 {
        return None;
    }
    let mut best: Option<(u16, usize, String)> = None;
    for (order, item) in raw.split(',').enumerate() {
        let mut parts = item.split(';');
        let Some(tag) = parts
            .next()
            .and_then(|value| clean_ascii_label(value.trim(), 24, "-"))
        else {
            continue;
        };
        if tag == "*" {
            continue;
        }
        let mut quality = 1_000;
        let mut valid = true;
        for parameter in parts {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("q") {
                let Ok(parsed) = value.trim().parse::<f32>() else {
                    valid = false;
                    break;
                };
                if !parsed.is_finite() || !(0.0..=1.0).contains(&parsed) {
                    valid = false;
                    break;
                } else {
                    quality = (parsed * 1_000.0).round() as u16;
                }
            }
        }
        if !valid || quality == 0 {
            continue;
        }
        if best.as_ref().is_none_or(|(best_quality, best_order, _)| {
            quality > *best_quality || (quality == *best_quality && order < *best_order)
        }) {
            best = Some((quality, order, tag));
        }
    }
    best.map(|(_, _, tag)| tag)
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
    let url = http_url(value)?;
    Some(url.origin().ascii_serialization())
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
        headers.insert(
            header::REFERER,
            HeaderValue::from_static(
                "https://benjisponge.com/thoughts/pesky-code?utm_source=letter&utm_campaign=hello&secret=discard",
            ),
        );
        headers.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("fr-CA;q=0.4, en-US;q=0.9, *;q=1"),
        );
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0) AppleWebKit Safari/605.1",
            ),
        );
        headers
    }

    fn payload() -> EventPayload {
        EventPayload {
            version: 2,
            id: Uuid::new_v4().to_string(),
            bootstrap_id: Uuid::new_v4().to_string(),
            kind: "pageview".to_string(),
            referrer: Some("https://www.google.com".to_string()),
            timezone: Some("America/New_York".to_string()),
            viewport_width: Some(390),
            engagement_ms: None,
            scroll_percent: None,
            lcp_ms: None,
            cls_milli: None,
            navigation_ms: None,
            target: None,
        }
    }

    #[test]
    fn derives_request_fields_and_coarsens_a_pageview() {
        let event = payload().validate(&headers(), 1_769_958_000).unwrap();
        assert_eq!(event.path, "/thoughts/pesky-code");
        assert_eq!(event.referrer_kind, "search");
        assert_eq!(event.referrer_host.as_deref(), Some("google.com"));
        assert_eq!(event.device_kind, "mobile");
        assert_eq!(event.viewport_kind, "phone");
        assert_eq!(event.operating_system, "iOS");
        assert_eq!(event.language.as_deref(), Some("en-US"));
        assert_eq!(event.utm_source.as_deref(), Some("letter"));
        assert_eq!(event.utm_campaign.as_deref(), Some("hello"));
        assert!(event.local_hour.is_some());
        assert!(event.local_weekday.is_some());
    }

    #[test]
    fn acquisition_referrer_must_already_be_privacy_reduced() {
        let event = payload().validate(&headers(), 1_769_958_000).unwrap();
        assert_eq!(event.referrer_host.as_deref(), Some("google.com"));
        assert_eq!(event.referrer_path, None);

        let mut internal = payload();
        internal.referrer = Some("https://benjisponge.com/resume".to_string());
        let event = internal.validate(&headers(), 1_769_958_000).unwrap();
        assert_eq!(event.referrer_kind, "internal");
        assert_eq!(event.referrer_path.as_deref(), Some("/resume"));

        let mut external_secret = payload();
        external_secret.referrer = Some("https://google.com/search?q=secret".to_string());
        assert_eq!(
            external_secret
                .validate(&headers(), 1_769_958_000)
                .unwrap_err(),
            "bad referrer"
        );

        let mut internal_secret = payload();
        internal_secret.referrer = Some("https://benjisponge.com/resume?private=yes".to_string());
        assert_eq!(
            internal_secret
                .validate(&headers(), 1_769_958_000)
                .unwrap_err(),
            "bad referrer"
        );

        // A same-host referrer off the trackable map (hidden page, 404)
        // degrades to direct: the pageview survives, the path is never stored.
        let mut hidden = payload();
        hidden.referrer = Some("https://benjisponge.com/motorcycles".to_string());
        let event = hidden.validate(&headers(), 1_769_958_000).unwrap();
        assert_eq!(event.referrer_kind, "direct");
        assert_eq!(event.referrer_host, None);
        assert_eq!(event.referrer_path, None);
    }

    #[test]
    fn current_page_must_be_one_same_origin_known_route() {
        let mut external = headers();
        external.insert(
            header::REFERER,
            HeaderValue::from_static("https://attacker.example/resume"),
        );
        assert_eq!(
            payload().validate(&external, 1_769_958_000).unwrap_err(),
            "bad current page"
        );

        let mut api = headers();
        api.insert(
            header::REFERER,
            HeaderValue::from_static("https://benjisponge.com/api/fitness/sets"),
        );
        assert_eq!(
            payload().validate(&api, 1_769_958_000).unwrap_err(),
            "bad current page"
        );

        let mut missing = headers();
        missing.insert(
            header::REFERER,
            HeaderValue::from_static("https://benjisponge.com/private-canary/alice"),
        );
        assert_eq!(
            payload().validate(&missing, 1_769_958_000).unwrap_err(),
            "bad current page"
        );

        let mut duplicate = headers();
        duplicate.append(
            header::REFERER,
            HeaderValue::from_static("https://benjisponge.com/resume"),
        );
        assert_eq!(
            payload().validate(&duplicate, 1_769_958_000).unwrap_err(),
            "bad current page"
        );
    }

    #[test]
    fn event_and_bootstrap_ids_are_canonical_v4_uuids() {
        let mut v1 = payload();
        v1.id = "550e8400-e29b-11d4-a716-446655440000".to_string();
        assert_eq!(
            v1.validate(&headers(), 1_769_958_000).unwrap_err(),
            "bad event id"
        );

        let mut old_contract = payload();
        old_contract.version = 1;
        assert_eq!(
            old_contract
                .validate(&headers(), 1_769_958_000)
                .unwrap_err(),
            "unsupported version"
        );

        let mut compact = payload();
        compact.id = compact.id.replace('-', "");
        assert_eq!(
            compact.validate(&headers(), 1_769_958_000).unwrap_err(),
            "bad event id"
        );

        let old_body = serde_json::json!({
            "version": 2,
            "id": Uuid::new_v4().to_string(),
            "bootstrap_id": Uuid::new_v4().to_string(),
            "kind": "pageview",
            "path": "/resume"
        });
        assert!(serde_json::from_value::<EventPayload>(old_body).is_err());
    }

    #[test]
    fn recognizes_regional_search_domains() {
        let mut regional = payload();
        regional.referrer = Some("https://www.google.co.uk".to_string());
        let event = regional.validate(&headers(), 1_769_958_000).unwrap();
        assert_eq!(event.referrer_kind, "search");
        assert_eq!(event.referrer_host.as_deref(), Some("google.co.uk"));
    }

    #[test]
    fn event_kinds_have_small_disjoint_payloads() {
        let mut pageview = payload();
        pageview.target = Some("https://example.com".to_string());
        assert_eq!(
            pageview.validate(&headers(), 1_769_958_000).unwrap_err(),
            "pageview has event-only fields"
        );

        let mut engagement = payload();
        engagement.kind = "engagement".to_string();
        engagement.referrer = None;
        engagement.timezone = None;
        engagement.viewport_width = None;
        engagement.engagement_ms = Some(12_000);
        engagement.scroll_percent = Some(75);
        let event = engagement.validate(&headers(), 1_769_958_000).unwrap();
        assert_eq!(event.engagement_seconds, Some(12));

        let mut outbound = payload();
        outbound.kind = "outbound".to_string();
        outbound.referrer = None;
        outbound.timezone = None;
        outbound.viewport_width = None;
        outbound.target = Some("https://example.com".to_string());
        let event = outbound.validate(&headers(), 1_769_958_000).unwrap();
        assert_eq!(event.target_host.as_deref(), Some("example.com"));

        let mut target_path = payload();
        target_path.kind = "outbound".to_string();
        target_path.referrer = None;
        target_path.timezone = None;
        target_path.viewport_width = None;
        target_path.target = Some("https://example.com/private".to_string());
        assert_eq!(
            target_path.validate(&headers(), 1_769_958_000).unwrap_err(),
            "target must be an origin"
        );
    }

    #[test]
    fn privacy_signals_are_exact_and_composable() {
        assert!(!tracking_opted_out(&headers()));

        for value in ["0", "01", "1foo"] {
            let mut signaled = headers();
            signaled.insert("sec-gpc", HeaderValue::from_str(value).unwrap());
            assert!(!tracking_opted_out(&signaled));
        }

        let mut gpc = headers();
        gpc.insert("sec-gpc", HeaderValue::from_static(" 1 "));
        assert!(tracking_opted_out(&gpc));

        let mut duplicate = headers();
        duplicate.append("sec-gpc", HeaderValue::from_static("0"));
        duplicate.append("sec-gpc", HeaderValue::from_static("1"));
        assert!(tracking_opted_out(&duplicate));

        let mut dnt = headers();
        dnt.insert("dnt", HeaderValue::from_static("1xyz"));
        assert!(tracking_opted_out(&dnt));
    }

    #[test]
    fn preferred_language_honors_quality_without_storing_the_list() {
        assert_eq!(preferred_language(&headers()).as_deref(), Some("en-US"));

        let mut none = headers();
        none.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("*;q=1, en;q=0"),
        );
        assert_eq!(preferred_language(&none), None);
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
