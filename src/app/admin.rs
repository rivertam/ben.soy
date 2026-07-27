//! Admin-only pages: `/admin` (the tool index) and `/admin/permissions`
//! (hidden-page grant management).
//!
//! The index is a rail of cards like `/interests`, fed by `ADMIN_TOOLS`; the
//! shell's admin-only footer link is its one listing. Permissions renders one
//! form set per registered hidden page — the granted emails, each with a
//! revoke button, and an add form. Grants whose page has since left
//! `HIDDEN_PAGES` surface in their own section so they can still be revoked.
//! Admin pages are tooling, not hidden pages: they stay out of every
//! registry, including `HIDDEN_PAGES` itself, and non-admins get the real
//! 404.
//!
//! Both POSTs repeat the admin identity check, require positive same-origin
//! evidence, and bound the body before parsing it — the forms are not an
//! authorization boundary. Semantic failures bounce back to the page as a
//! `?notice=` code instead of a bare error body, so the no-JS forms stay
//! pleasant; mechanical failures (wrong content type, oversize body) get
//! plain 4xxs, which only non-browser callers ever see. Responses that
//! redirect are hand-built `Ok(303)`s like the auth routes' — not because
//! these touch cookies, but so every branch carries `no-store`.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, Response, StatusCode, header, headers, page, query_params,
        redirect, route, to_bytes,
    },
    view::view,
};

use crate::components::{index_card, page_head, shell};
use crate::content::access::{self, HIDDEN_PAGES, HiddenPage, HiddenPageGrant, is_admin};

use super::analytics::is_same_origin;
use super::diary;
use super::login::viewer;
use super::not_found::not_found_page;

const INDEX_PATH: &str = "/admin";
const INDEX_LOGIN_REDIRECT: &str = "/login?next=%2Fadmin";
const PAGE_PATH: &str = "/admin/permissions";
const LOGIN_REDIRECT: &str = "/login?next=%2Fadmin%2Fpermissions";
const BODY_LIMIT_BYTES: usize = 4 * 1024;
const NO_STORE: &str = "no-store";

/// An `/admin` index card. Add an entry when a new admin page lands; the
/// index renders these exactly like the interests rail.
struct AdminTool {
    stamp: &'static str,
    href: &'static str,
    title: &'static str,
    teaser: &'static str,
}

static ADMIN_TOOLS: [AdminTool; 2] = [
    AdminTool {
        stamp: "permissions",
        href: PAGE_PATH,
        title: "Permissions",
        teaser: "Who may open which hidden page. Grants and revocations apply on \
                 the next request.",
    },
    AdminTool {
        stamp: "diary",
        href: diary::PATH,
        title: "Diary",
        teaser: "Completely private, timestamped entries. Deliberately not a \
                 hidden page, so it can never be granted.",
    },
];

#[page("/admin")]
async fn admin_index(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return Err(redirect(INDEX_LOGIN_REDIRECT).into());
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: INDEX_PATH)
        };
    }
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: "Admin",
            active: "",
            runtime: false,
            analytics: false,
            page_head(stamp: "index", title: "Admin", lede: "The back office.")
            <section class="mt-14 space-y-10">
                for tool in ADMIN_TOOLS.iter() {
                    index_card(
                        stamp: tool.stamp,
                        href: tool.href.to_string(),
                        title: tool.title,
                        teaser: tool.teaser
                    )
                }
            </section>
        )
    }
}

const META_LABEL: &str =
    "font-meta text-[0.6875rem] leading-normal tracking-[0.13em] uppercase text-muted";
const CONTROL: &str = "w-full min-w-0 h-[2.65rem] px-3 py-[0.65rem] text-ink bg-page \
     border border-hairline rounded-[0.2rem] font-body text-sm leading-[1.2] outline-none \
     placeholder:text-muted placeholder:opacity-100 \
     hover:border-[color-mix(in_srgb,var(--color-ink2)_45%,var(--color-hairline))] \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";

#[query_params(error = redirect("?"))]
struct PermissionsQuery {
    notice: Option<String>,
}

#[page("/admin/permissions")]
async fn permissions(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return Err(redirect(LOGIN_REDIRECT).into());
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: PAGE_PATH)
        };
    }
    let query = query_params::<PermissionsQuery>(cx)?;
    let notice = query.notice.as_deref().map(|code| match code {
        "granted" => "Granted — it applies on their next request.",
        "revoked" => "Revoked — it applies on their next request.",
        "invalid" => "That didn't validate; nothing changed.",
        "unavailable" => "The grant store didn't answer; nothing changed.",
        _ => "Nothing changed.",
    });
    let (sections, orphans, store_ok) = match access::grants(app_context::<Data>(cx)).await {
        Ok(all) => {
            let (sections, orphans) = arrange(all);
            (sections, orphans, true)
        }
        Err(error) => {
            log_failure("list", &error);
            (Vec::new(), Vec::new(), false)
        }
    };
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: "Permissions",
            active: "",
            runtime: false,
            analytics: false,
            page_head(
                stamp: "admin",
                title: "Permissions",
                lede: "Who may open which hidden page. Grants apply on the next \
                       request; so do revocations."
            )
            if let Some(message) = notice {
                <p class="mt-6 max-w-prose border-l-2 border-oxide pl-3 font-meta text-sm text-ink2">
                    (message)
                </p>
            }
            if !store_ok {
                <p class="mt-8 max-w-prose text-ink2">
                    "The grant store is unreachable, so nothing can be listed or "
                    "changed right now. Hidden pages are failing closed in the "
                    "meantime — friends are locked out, you are not."
                </p>
            }
            if store_ok {
                for (hidden, emails) in sections.iter() {
                    <section class="mt-12 max-w-prose">
                        <header class="flex flex-wrap items-baseline justify-between gap-x-4">
                            <h2 class="font-display text-2xl font-bold tracking-tight">
                                (hidden.title)
                            </h2>
                            <p class="font-meta text-xs text-muted">(hidden.path)</p>
                        </header>
                        if emails.is_empty() {
                            <p class="mt-3 text-sm text-muted">
                                "No one is granted — only you can open it."
                            </p>
                        } else {
                            <ul class="mt-3 divide-y divide-hairline border-y border-hairline">
                                for email in emails.iter() {
                                    <li class="flex items-baseline justify-between gap-4 py-2">
                                        <span class="min-w-0 break-all font-meta text-sm text-ink2">
                                            (email.as_str())
                                        </span>
                                        <form method="post" action="/admin/permissions/revoke">
                                            <input type="hidden" name="path" value=(hidden.path)>
                                            <input type="hidden" name="email" value=(email.as_str())>
                                            <button
                                                type="submit"
                                                class="quiet-link cursor-pointer font-meta text-xs"
                                            >"revoke"</button>
                                        </form>
                                    </li>
                                }
                            </ul>
                        }
                        <form
                            method="post"
                            action="/admin/permissions/grant"
                            class="mt-4 flex items-end gap-3"
                        >
                            <input type="hidden" name="path" value=(hidden.path)>
                            <label
                                class="flex min-w-0 grow flex-col gap-[0.35rem]"
                                for=(format!("grant-{}", hidden.stamp))
                            >
                                <span class=(META_LABEL)>"grant an email"</span>
                                <input
                                    class=(CONTROL)
                                    id=(format!("grant-{}", hidden.stamp))
                                    name="email"
                                    type="email"
                                    required=""
                                    placeholder="friend@example.com"
                                >
                            </label>
                            <button
                                type="submit"
                                class="oxlink cursor-pointer pb-[0.65rem] font-meta text-sm whitespace-nowrap"
                            >"grant →"</button>
                        </form>
                    </section>
                }
                if !orphans.is_empty() {
                    <section class="mt-14 max-w-prose">
                        <h2 class="font-display text-2xl font-bold tracking-tight">
                            "No longer registered"
                        </h2>
                        <p class="mt-2 text-sm text-ink2">
                            "Grants for pages that have left "
                            <span class="font-meta">"HIDDEN_PAGES"</span>
                            ". They do nothing while the page is unregistered — "
                            "revoke to tidy, or restore the page to honor them again."
                        </p>
                        for (path, emails) in orphans.iter() {
                            <p class="mt-5 font-meta text-xs text-muted">(path.as_str())</p>
                            <ul class="mt-1 divide-y divide-hairline border-y border-hairline">
                                for email in emails.iter() {
                                    <li class="flex items-baseline justify-between gap-4 py-2">
                                        <span class="min-w-0 break-all font-meta text-sm text-ink2">
                                            (email.as_str())
                                        </span>
                                        <form method="post" action="/admin/permissions/revoke">
                                            <input type="hidden" name="path" value=(path.as_str())>
                                            <input type="hidden" name="email" value=(email.as_str())>
                                            <button
                                                type="submit"
                                                class="quiet-link cursor-pointer font-meta text-xs"
                                            >"revoke"</button>
                                        </form>
                                    </li>
                                }
                            </ul>
                        }
                    </section>
                }
            }
        )
    }
}

#[route(POST "/admin/permissions/grant")]
async fn grant(cx: &Cx, body: Body) -> Result<Response> {
    let form = match gate(cx, body).await {
        Ok(form) => form,
        Err(response) => return Ok(response),
    };
    let Some(email) = access::normalize_email(&form.email) else {
        return Ok(back("invalid"));
    };
    if access::hidden_page(&form.path).is_none() {
        return Ok(back("invalid"));
    }
    match access::grant(app_context::<Data>(cx), &form.path, &email, epoch_seconds()).await {
        Ok(()) => Ok(back("granted")),
        Err(error) => {
            log_failure("grant", &error);
            Ok(back("unavailable"))
        }
    }
}

#[route(POST "/admin/permissions/revoke")]
async fn revoke(cx: &Cx, body: Body) -> Result<Response> {
    let form = match gate(cx, body).await {
        Ok(form) => form,
        Err(response) => return Ok(response),
    };
    let Some(email) = access::normalize_email(&form.email) else {
        return Ok(back("invalid"));
    };
    // Plausibility, not registration: orphaned grants must stay revocable.
    if !access::plausible_grant_path(&form.path) {
        return Ok(back("invalid"));
    }
    match access::revoke(app_context::<Data>(cx), &form.path, &email).await {
        Ok(()) => Ok(back("revoked")),
        Err(error) => {
            log_failure("revoke", &error);
            Ok(back("unavailable"))
        }
    }
}

/// Registry-ordered sections plus any grants for de-registered pages, emails
/// sorted within each.
type Sections = Vec<(&'static HiddenPage, Vec<String>)>;
type Orphans = Vec<(String, Vec<String>)>;

fn arrange(grants: Vec<HiddenPageGrant>) -> (Sections, Orphans) {
    let mut by_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // `stored`, not `grant`: the #[route] macro above makes `grant` a const.
    for stored in grants {
        by_path
            .entry(stored.page_path)
            .or_default()
            .push(stored.email);
    }
    let mut sections = Vec::new();
    for hidden in HIDDEN_PAGES.iter() {
        let mut emails = by_path.remove(hidden.path).unwrap_or_default();
        emails.sort();
        emails.dedup();
        sections.push((hidden, emails));
    }
    let orphans = by_path
        .into_iter()
        .map(|(path, mut emails)| {
            emails.sort();
            emails.dedup();
            (path, emails)
        })
        .collect();
    (sections, orphans)
}

struct PermissionForm {
    path: String,
    email: String,
}

/// The shared preamble both POSTs run before believing anything in the body.
async fn gate(cx: &Cx, body: Body) -> std::result::Result<PermissionForm, Response> {
    let Some(current) = viewer(cx) else {
        return Err(see_other(LOGIN_REDIRECT));
    };
    if !is_admin(&current.email) {
        return Err(plain(StatusCode::NOT_FOUND, "not found"));
    }
    if !is_same_origin(headers(cx)) {
        return Err(plain(StatusCode::FORBIDDEN, "forbidden"));
    }
    if !is_form_content_type(headers(cx)) {
        return Err(plain(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        ));
    }
    let bytes = match to_bytes(body, BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return Err(plain(StatusCode::PAYLOAD_TOO_LARGE, "form is too large")),
    };
    parse_permission_form(&bytes).ok_or_else(|| plain(StatusCode::BAD_REQUEST, "bad form"))
}

/// Exactly one `path` and one `email` field, nothing else. Invalid UTF-8
/// decodes lossily and then fails the printable-ASCII validation upstream.
fn parse_permission_form(body: &[u8]) -> Option<PermissionForm> {
    let mut path = None;
    let mut email = None;
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "path" if path.is_none() => path = Some(value.into_owned()),
            "email" if email.is_none() => email = Some(value.into_owned()),
            _ => return None,
        }
    }
    Some(PermissionForm {
        path: path?,
        email: email?,
    })
}

fn is_form_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Bounce back to the page with a static notice code — never echoed input.
fn back(notice: &'static str) -> Response {
    see_other(&format!("{PAGE_PATH}?notice={notice}"))
}

fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, NO_STORE)
        .body(Body::from("see other"))
        .expect("static locations are valid headers")
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(message))
        .expect("static headers")
}

fn log_failure(step: &str, error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "hidden-page grant admin failed",
            "step": step,
            "error": error,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_wants_exactly_path_and_email_once() {
        let form = parse_permission_form(b"path=%2Fmotorcycles&email=Alice%40Gmail.com").unwrap();
        assert_eq!(form.path, "/motorcycles");
        assert_eq!(form.email, "Alice@Gmail.com");

        let reversed = parse_permission_form(b"email=a%40b.co&path=%2Fx").unwrap();
        assert_eq!(reversed.path, "/x");
        assert_eq!(reversed.email, "a@b.co");

        assert!(parse_permission_form(b"").is_none());
        assert!(parse_permission_form(b"path=%2Fx").is_none());
        assert!(parse_permission_form(b"email=a%40b.co").is_none());
        assert!(parse_permission_form(b"path=%2Fx&path=%2Fy&email=a%40b.co").is_none());
        assert!(parse_permission_form(b"path=%2Fx&email=a%40b.co&submit=Grant").is_none());
    }

    #[test]
    fn form_content_type_is_required_shape() {
        let mut headers = HeaderMap::new();
        assert!(!is_form_content_type(&headers));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("Application/X-Www-Form-Urlencoded; charset=UTF-8"),
        );
        assert!(is_form_content_type(&headers));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!is_form_content_type(&headers));
    }

    /// Admin tooling is unlisted, exactly like hidden pages: out of the route
    /// registry (so the 404's index and sitemap never mention it) and off the
    /// analytics-trackable prefixes.
    #[test]
    fn admin_pages_stay_unlisted() {
        for path in [INDEX_PATH, PAGE_PATH] {
            assert!(
                !crate::content::routes::site_routes().contains(&path.to_string()),
                "{path} leaked into site_routes()"
            );
            assert!(!crate::content::routes::is_trackable_route(path));
        }
        for tool in ADMIN_TOOLS.iter() {
            // The diary is admin-only tooling at its own path; everything
            // else lives under /admin/. Either way a tool must stay out of
            // the public registries and off the analytics-trackable routes.
            assert!(
                tool.href.starts_with("/admin/") || tool.href == diary::PATH,
                "{} points outside /admin/",
                tool.href
            );
            assert!(
                !crate::content::routes::site_routes().contains(&tool.href.to_string()),
                "{} leaked into site_routes()",
                tool.href
            );
            assert!(!crate::content::routes::is_trackable_route(tool.href));
        }
        assert_eq!(ADMIN_TOOLS[0].href, PAGE_PATH);
        assert_eq!(
            LOGIN_REDIRECT,
            format!("/login?next={}", crate::util::urlencode(PAGE_PATH))
        );
        assert_eq!(
            INDEX_LOGIN_REDIRECT,
            format!("/login?next={}", crate::util::urlencode(INDEX_PATH))
        );
    }
}
