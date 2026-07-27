//! Who may see hidden pages. Identity comes from Google sign-in
//! (`src/app/login.rs`); this module is the authorization side.
//!
//! Allowlists live in the `HIDDEN_PAGE_ACCESS` env var, NEVER in this file:
//! the repo is public, so a committed grant would publish a friend's email
//! address to git history forever. Format, entries `;`-separated, emails
//! `,`-separated:
//!
//! ```text
//! HIDDEN_PAGE_ACCESS=/motorcycles:alice@gmail.com,bob@gmail.com;/garage:carol@example.com
//! ```
//!
//! The admin sees every hidden page without being listed. Checks read the
//! env on every request, so editing the Railway variable (which redeploys)
//! grants or revokes access immediately — no session state to invalidate.
//! Hidden pages deliberately stay out of `INTERESTS`/`POSTS`/`site_routes()`:
//! the nav, indexes, feed, and 404 never mention them.

/// Already public as the repo's commit author email, so hardcoding it leaks
/// nothing new. Friends' emails are a different story — env only.
pub const ADMIN_EMAIL: &str = "ben.b@digichem.com";

/// A hidden page's display entry: what allowlisted viewers see when the nav
/// dropdown and the interests index populate for them. Paths and copy are
/// committed — the page modules are public source anyway; only WHO may view
/// stays in the env.
pub struct HiddenPage {
    pub path: &'static str,
    pub stamp: &'static str,
    pub title: &'static str,
    pub teaser: &'static str,
}

pub static HIDDEN_PAGES: [HiddenPage; 1] = [HiddenPage {
    path: "/motorcycles",
    stamp: "motorcycles",
    title: "Motorcycles",
    teaser: "The garage log: bikes, routes, and wrenching notes. Shared, not published.",
}];

/// The hidden pages `email` may view, in registry order. This is the whole
/// "only shows up if I allowlist you" surface: anonymous visitors never reach
/// this call, everyone else sees exactly their grants (the admin, all of it).
pub fn visible_pages(email: &str) -> impl Iterator<Item = &'static HiddenPage> + '_ {
    HIDDEN_PAGES
        .iter()
        .filter(move |page| may_view(email, page.path))
}

const ACCESS_VAR: &str = "HIDDEN_PAGE_ACCESS";

/// Whether `email` (a verified Google account email) may view the hidden
/// page at `path`.
pub fn may_view(email: &str, path: &str) -> bool {
    is_admin(email) || raw().is_some_and(|raw| may_view_in(&raw, email, path))
}

/// Whether `email` appears anywhere in the allowlists. The login callback
/// refuses to mint a viewer cookie for accounts with no access at all, so
/// strangers who find `/login` end up holding nothing.
pub fn known_viewer(email: &str) -> bool {
    is_admin(email) || raw().is_some_and(|raw| known_in(&raw, email))
}

fn is_admin(email: &str) -> bool {
    email.eq_ignore_ascii_case(ADMIN_EMAIL)
}

fn raw() -> Option<String> {
    std::env::var(ACCESS_VAR).ok().filter(|v| !v.is_empty())
}

fn may_view_in(raw: &str, email: &str, path: &str) -> bool {
    entries(raw).any(|(entry_path, emails)| entry_path == path && allows(emails, email))
}

fn known_in(raw: &str, email: &str) -> bool {
    entries(raw).any(|(_, emails)| allows(emails, email))
}

fn entries(raw: &str) -> impl Iterator<Item = (&str, &str)> {
    raw.split(';')
        .filter_map(|entry| entry.trim().split_once(':'))
        .map(|(path, emails)| (path.trim(), emails))
}

fn allows(emails: &str, email: &str) -> bool {
    emails.split(',').map(str::trim).any(|allowed| {
        // An empty allowed entry must never match an empty claim.
        !allowed.is_empty() && allowed.eq_ignore_ascii_case(email)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "/motorcycles:alice@gmail.com, Bob@Gmail.com ;/garage:carol@example.com";

    #[test]
    fn admin_sees_every_hidden_page() {
        assert!(may_view(ADMIN_EMAIL, "/motorcycles"));
        assert!(may_view(
            &ADMIN_EMAIL.to_uppercase(),
            "/not-registered-anywhere"
        ));
        assert!(known_viewer(ADMIN_EMAIL));
    }

    #[test]
    fn grants_are_per_path_and_case_insensitive() {
        assert!(may_view_in(RAW, "alice@gmail.com", "/motorcycles"));
        assert!(may_view_in(RAW, "ALICE@gmail.com", "/motorcycles"));
        assert!(may_view_in(RAW, "bob@gmail.com", "/motorcycles"));
        assert!(may_view_in(RAW, "carol@example.com", "/garage"));
        assert!(!may_view_in(RAW, "alice@gmail.com", "/garage"));
        assert!(!may_view_in(RAW, "carol@example.com", "/motorcycles"));
    }

    #[test]
    fn known_viewer_spans_all_entries_and_rejects_strangers() {
        assert!(known_in(RAW, "alice@gmail.com"));
        assert!(known_in(RAW, "carol@example.com"));
        assert!(!known_in(RAW, "stranger@example.com"));
        assert!(!may_view_in(RAW, "stranger@example.com", "/motorcycles"));
    }

    #[test]
    fn hidden_pages_stay_out_of_the_public_registries() {
        for page in &HIDDEN_PAGES {
            assert!(page.path.starts_with('/'), "{} must be absolute", page.path);
            assert_eq!(page.path, format!("/{}", page.stamp), "stamp mirrors path");
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

    /// With no allowlist env in the test process, grants come from the admin
    /// rule alone: the admin sees every registered page, strangers see none.
    #[test]
    fn visible_pages_follow_the_admin_rule() {
        assert_eq!(visible_pages(ADMIN_EMAIL).count(), HIDDEN_PAGES.len());
        assert_eq!(visible_pages("stranger@example.com").count(), 0);
    }

    #[test]
    fn malformed_entries_grant_nothing() {
        assert!(!may_view_in("", "alice@gmail.com", "/motorcycles"));
        assert!(!may_view_in("garbage", "alice@gmail.com", "/motorcycles"));
        assert!(!may_view_in("/motorcycles:", "", "/motorcycles"));
        assert!(!may_view_in(";;;", "alice@gmail.com", "/motorcycles"));
    }
}
