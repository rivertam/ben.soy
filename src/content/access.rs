//! Who may see hidden pages. Identity comes from Google sign-in
//! (`src/app/login.rs`); this module is the authorization side.
//!
//! Grants live in the `hidden_page_grants` table, NEVER in this file: the
//! repo is public, so a committed grant would publish a friend's email
//! address to git history forever. The admin manages them at
//! `/admin/permissions` (`src/app/admin.rs`); checks query the database on
//! the request that needs them, so a grant or revocation applies on the very
//! next request — no session state to invalidate, no redeploy. When the
//! database is unreachable, checks fail closed: hidden pages are admin-only
//! (the admin rule is a constant comparison and never needs the database).
//! Hidden pages deliberately stay out of `INTERESTS`/`POSTS`/`site_routes()`:
//! the nav, indexes, feed, and 404 never mention them.

use std::collections::HashSet;

use benjisponge::data::Data;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use surrealdb::types::SurrealValue;

/// The single app-wide administrator. This address is intentionally committed
/// because it defines privileged behavior; friends' hidden-page grants remain
/// database-only.
pub const ADMIN_EMAIL: &str = "ben.m.berman@gmail.com";

/// A hidden page's display entry: what allowlisted viewers see when the nav
/// dropdown and the interests index populate for them. Paths and copy are
/// committed — the page modules are public source anyway; only WHO may view
/// stays in the database. `/admin/permissions` derives its grant forms from
/// this registry, so a new hidden page gets its form set for free.
pub struct HiddenPage {
    pub path: &'static str,
    pub stamp: &'static str,
    pub title: &'static str,
    pub teaser: &'static str,
}

pub static HIDDEN_PAGES: [HiddenPage; 2] = [
    HiddenPage {
        path: "/motorcycles",
        stamp: "motorcycles",
        title: "Motorcycles",
        teaser: "The garage log: bikes, routes, and wrenching notes. Shared, not published.",
    },
    HiddenPage {
        path: "/podrick",
        stamp: "podrick",
        title: "Pants Off Podrick",
        teaser: "A Discord bot for the Daniel Aficionados server",
    },
];

/// The registered hidden page at `path`, if any. The admin page validates
/// grant targets against this — you can't grant a page that doesn't exist.
pub fn hidden_page(path: &str) -> Option<&'static HiddenPage> {
    HIDDEN_PAGES.iter().find(|page| page.path == path)
}

/// One stored grant: `email` may view the hidden page at `page_path`. The
/// record id is [`grant_id`] of the pair, so the pair is the identity.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct HiddenPageGrant {
    pub id: String,
    pub page_path: String,
    pub email: String,
    pub granted_at: i64,
}

pub fn is_admin(email: &str) -> bool {
    email.eq_ignore_ascii_case(ADMIN_EMAIL)
}

/// Whether `email` (a verified Google account email) may view the hidden
/// page at `path`. A database failure denies (and logs) rather than erring
/// the page — the admin rule keeps working regardless.
pub async fn may_view(data: &Data, email: &str, path: &str) -> bool {
    if is_admin(email) {
        return true;
    }
    match granted_paths(data, email).await {
        Ok(paths) => paths.contains(path),
        Err(error) => {
            log_failure("may_view", &error);
            false
        }
    }
}

/// Whether `email` holds any grant at all. The login callback refuses to
/// mint a viewer cookie for accounts with no access, so strangers who find
/// `/login` end up holding nothing.
pub async fn known_viewer(data: &Data, email: &str) -> bool {
    if is_admin(email) {
        return true;
    }
    match granted_paths(data, email).await {
        Ok(paths) => !paths.is_empty(),
        Err(error) => {
            log_failure("known_viewer", &error);
            false
        }
    }
}

/// The hidden pages `email` may view, in registry order. This is the whole
/// "only shows up if I allowlist you" surface: anonymous visitors never reach
/// this call, everyone else sees exactly their grants.
pub async fn visible_pages(data: &Data, email: &str) -> Vec<&'static HiddenPage> {
    if is_admin(email) {
        return HIDDEN_PAGES.iter().collect();
    }
    match granted_paths(data, email).await {
        Ok(paths) => visible_from(&paths),
        Err(error) => {
            log_failure("visible_pages", &error);
            Vec::new()
        }
    }
}

fn visible_from(granted: &HashSet<String>) -> Vec<&'static HiddenPage> {
    HIDDEN_PAGES
        .iter()
        .filter(|page| granted.contains(page.path))
        .collect()
}

async fn granted_paths(data: &Data, email: &str) -> Result<HashSet<String>, String> {
    let db = data.db().await.map_err(|error| error.to_string())?;
    let mut response = db
        .query("SELECT VALUE page_path FROM hidden_page_grants WHERE email = $email")
        .bind(("email", email.to_ascii_lowercase()))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let paths: Vec<String> = response.take(0).map_err(|error| error.to_string())?;
    Ok(paths.into_iter().collect())
}

/// Every stored grant, unordered — the admin page groups and sorts in Rust.
/// Unlike the boolean checks, the error surfaces: the admin page must say
/// "store unreachable", never render every list empty as if all grants were
/// revoked.
pub async fn grants(data: &Data) -> Result<Vec<HiddenPageGrant>, String> {
    let db = data.db().await.map_err(|error| error.to_string())?;
    let mut response = db
        .query("SELECT *, record::id(id) AS id FROM hidden_page_grants")
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    response.take(0).map_err(|error| error.to_string())
}

/// Store a grant. Idempotent: the deterministic record key makes re-granting
/// an UPSERT of the same record, and `granted_at` keeps its original value.
/// `email` must already be [`normalize_email`]d and `path` registered — the
/// admin routes validate before calling.
pub async fn grant(data: &Data, path: &str, email: &str, granted_at: i64) -> Result<(), String> {
    let db = data.db().await.map_err(|error| error.to_string())?;
    db.query(
        "UPSERT ONLY type::record('hidden_page_grants', $id)
             SET page_path = $path,
                 email = $email,
                 granted_at = granted_at ?? $granted_at",
    )
    .bind(("id", grant_id(path, email)))
    .bind(("path", path.to_string()))
    .bind(("email", email.to_string()))
    .bind(("granted_at", granted_at))
    .await
    .map_err(|error| error.to_string())?
    .check()
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Delete a grant. Idempotent, and deletes by pair rather than record key so
/// it also clears any row a non-deterministic writer might have left behind.
pub async fn revoke(data: &Data, path: &str, email: &str) -> Result<(), String> {
    let db = data.db().await.map_err(|error| error.to_string())?;
    db.query("DELETE hidden_page_grants WHERE page_path = $path AND email = $email")
        .bind(("path", path.to_string()))
        .bind(("email", email.to_string()))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// The deterministic record key for a (page, email) pair: sha-256 of both,
/// newline-separated (validated inputs are printable ASCII, so the separator
/// is unambiguous). A hex key sidesteps record-id escaping for keys that
/// would otherwise contain `/`, `@`, and `.`.
pub fn grant_id(path: &str, email: &str) -> String {
    Sha256::digest(format!("{path}\n{email}"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Trimmed, lowercased, and shape-checked — the only form emails are stored
/// or compared in. `None` means "not an address we would ever store". The
/// shape check (printable ASCII, exactly one `@`, dotted domain) mirrors the
/// schema ASSERT; it exists to catch typos at the admin form, not to
/// implement RFC 5322.
pub fn normalize_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_ascii_lowercase();
    if email.len() > 254 || !email.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return None;
    }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.contains('@') {
        return None;
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return None;
    }
    Some(email)
}

/// The loose shape check for a grant's page path — enough to store or delete
/// by. Deliberately weaker than "is registered" so grants orphaned by a page
/// leaving [`HIDDEN_PAGES`] can still be revoked; granting additionally
/// requires [`hidden_page`] to hit.
pub fn plausible_grant_path(path: &str) -> bool {
    (2..=120).contains(&path.len())
        && path.starts_with('/')
        && !path.starts_with("//")
        && path.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn log_failure(check: &str, error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "hidden-page grant lookup failed",
            "check": check,
            "error": error,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_identity_is_case_insensitive_and_exclusive() {
        assert!(is_admin(ADMIN_EMAIL));
        assert!(is_admin(&ADMIN_EMAIL.to_uppercase()));
        assert!(!is_admin("alice@gmail.com"));
        assert!(!is_admin(""));
    }

    #[test]
    fn visibility_follows_granted_paths_in_registry_order() {
        let none = HashSet::new();
        assert!(visible_from(&none).is_empty());

        let granted: HashSet<String> = [
            "/podrick".to_string(),
            "/motorcycles".to_string(),
            "/garage".to_string(),
        ]
        .into();
        let visible = visible_from(&granted);
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].path, "/motorcycles");
        assert_eq!(visible[1].path, "/podrick");
    }

    #[test]
    fn registered_pages_resolve_and_strangers_do_not() {
        assert_eq!(hidden_page("/motorcycles").unwrap().stamp, "motorcycles");
        assert_eq!(hidden_page("/podrick").unwrap().stamp, "podrick");
        assert!(hidden_page("/garage").is_none());
        assert!(hidden_page("motorcycles").is_none());
    }

    #[test]
    fn emails_normalize_to_lowercase_or_not_at_all() {
        assert_eq!(
            normalize_email("  Alice@Gmail.Com "),
            Some("alice@gmail.com".to_string())
        );
        assert_eq!(
            normalize_email("a.b+c@example.co.uk"),
            Some("a.b+c@example.co.uk".to_string())
        );
        for bad in [
            "",
            "   ",
            "plainaddress",
            "@example.com",
            "alice@",
            "alice@nodot",
            "alice@.leading.dot",
            "alice@trailing.dot.",
            "two@at@example.com",
            "spaced out@example.com",
            "unicode-héllo@example.com",
        ] {
            assert_eq!(normalize_email(bad), None, "accepted {bad:?}");
        }
        let long = format!("{}@example.com", "a".repeat(255));
        assert_eq!(normalize_email(&long), None);
    }

    #[test]
    fn grant_paths_are_absolute_printable_and_bounded() {
        assert!(plausible_grant_path("/motorcycles"));
        assert!(plausible_grant_path("/a"));
        for bad in [
            "",
            "/",
            "//evil",
            "motorcycles",
            "/with space",
            "/tab\there",
        ] {
            assert!(!plausible_grant_path(bad), "accepted {bad:?}");
        }
        assert!(!plausible_grant_path(&format!("/{}", "a".repeat(120))));
    }

    #[test]
    fn grant_ids_are_stable_hex_and_pair_distinct() {
        let id = grant_id("/motorcycles", "alice@gmail.com");
        assert_eq!(id, grant_id("/motorcycles", "alice@gmail.com"));
        assert_eq!(id.len(), 64);
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(id, grant_id("/garage", "alice@gmail.com"));
        assert_ne!(id, grant_id("/motorcycles", "bob@gmail.com"));
    }

    #[test]
    fn hidden_pages_stay_out_of_the_public_registries() {
        for page in &HIDDEN_PAGES {
            assert!(page.path.starts_with('/'), "{} must be absolute", page.path);
            assert_eq!(page.path, format!("/{}", page.stamp), "stamp mirrors path");
            assert!(
                plausible_grant_path(page.path),
                "{} would fail its own grant validation",
                page.path
            );
            assert!(
                !crate::content::routes::site_routes().contains(&page.path.to_string()),
                "{} leaked into site_routes()",
                page.path
            );
            assert!(
                crate::content::interests::INTERESTS
                    .iter()
                    .all(|i| format!("/{}", i.slug) != page.path),
                "{} collides with a public interest",
                page.path
            );
            // A trackable path would let the analytics pipeline record the
            // hidden page as a referrer of public pageviews — hidden pages
            // must never live under /felix/, /swing/, or /lifting/.
            assert!(
                !crate::content::routes::is_trackable_route(page.path),
                "{} is analytics-trackable",
                page.path
            );
        }
    }
}
