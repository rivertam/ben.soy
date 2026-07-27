//! Bearer-token check for the private write endpoints.
//!
//! Port of the old Worker's `auth.ts` (since deleted): the header must match `^Bearer ([^\s]+)$`,
//! an unset/empty secret closes the write path, and both sides are SHA-256
//! hashed before a constant-time comparison so neither mismatch position nor
//! length leaks timing.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub fn bearer_authorized(authorization: Option<&str>, expected: Option<&str>) -> bool {
    let Some(expected) = expected.filter(|secret| !secret.is_empty()) else {
        return false;
    };
    let Some(header) = authorization else {
        return false;
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return false;
    };
    if token.is_empty() || token.contains(|c: char| c.is_whitespace()) {
        return false;
    }
    secrets_match(token, expected)
}

/// Constant-time equality for short secrets (bearer tokens, OAuth `state`
/// values, token audiences). An empty expectation never matches.
pub fn secrets_match(provided: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    let provided_hash = Sha256::digest(provided.as_bytes());
    let expected_hash = Sha256::digest(expected.as_bytes());
    provided_hash.ct_eq(&expected_hash).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_exact_token() {
        assert!(bearer_authorized(Some("Bearer sekrit"), Some("sekrit")));
    }

    #[test]
    fn rejects_wrong_missing_or_malformed() {
        assert!(!bearer_authorized(Some("Bearer wrong"), Some("sekrit")));
        assert!(!bearer_authorized(None, Some("sekrit")));
        assert!(!bearer_authorized(Some("sekrit"), Some("sekrit")));
        assert!(!bearer_authorized(Some("bearer sekrit"), Some("sekrit")));
        assert!(!bearer_authorized(Some("Bearer  sekrit"), Some("sekrit")));
        assert!(!bearer_authorized(Some("Bearer sek rit"), Some("sekrit")));
        assert!(!bearer_authorized(Some("Bearer "), Some("sekrit")));
    }

    #[test]
    fn unset_or_empty_secret_closes_the_write_path() {
        assert!(!bearer_authorized(Some("Bearer sekrit"), None));
        assert!(!bearer_authorized(Some("Bearer "), Some("")));
        assert!(!bearer_authorized(Some("Bearer x"), Some("")));
    }
}
