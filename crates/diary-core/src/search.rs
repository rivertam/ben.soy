//! Fuzzy diary search — one matcher for the server page and offline SSR.
//!
//! Ranking is fzf-style subsequence scoring over entry bodies (synced rows
//! only; hosts filter before calling). Empty/whitespace queries are not a
//! search. The needle is capped so a malicious or accidental paste cannot
//! dominate ranking cost.

use crate::entry::DiaryEntry;
use crate::store::PAGE_SIZE;

/// Cap on the query the UI accepts. Longer input is truncated, not rejected,
/// so a paste still searches something useful.
pub const MAX_NEEDLE_CHARS: usize = 100;

/// Approximate character budget for a hit-list snippet.
pub const SNIPPET_CHARS: usize = 120;

/// Trim and cap a raw `q` value. `None` means "not a search" (show the
/// normal transcript).
pub fn normalize_query(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (index, ch) in trimmed.chars().enumerate() {
        if index >= MAX_NEEDLE_CHARS {
            break;
        }
        out.push(ch);
    }
    if out.is_empty() { None } else { Some(out) }
}

/// One ranked hit. Hosts turn this into markup; the score is kept for tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankedHit {
    pub entry: DiaryEntry,
    pub score: i64,
}

/// Score every entry whose body fuzzy-matches `needle`, best score first,
/// then newest `written_at`, then id descending for stability.
pub fn rank(needle: &str, entries: impl IntoIterator<Item = DiaryEntry>) -> Vec<RankedHit> {
    let mut hits: Vec<RankedHit> = entries
        .into_iter()
        .filter_map(|entry| score(needle, &entry.body).map(|score| RankedHit { entry, score }))
        .collect();
    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then(right.entry.written_at.cmp(&left.entry.written_at))
            .then(right.entry.id.as_str().cmp(left.entry.id.as_str()))
    });
    hits
}

/// One `PAGE_SIZE` window of an already-ranked hit list (1-based page).
pub fn page_hits(hits: &[RankedHit], page_number: usize) -> Vec<&RankedHit> {
    let page_number = page_number.max(1);
    let start = page_number.saturating_sub(1).saturating_mul(PAGE_SIZE);
    hits.iter().skip(start).take(PAGE_SIZE).collect()
}

/// Truncate `body` for the hit list, preferring a window around the first
/// fuzzy match when the text is long.
pub fn snippet(needle: &str, body: &str) -> String {
    let collapsed = collapse_whitespace(body);
    if collapsed.chars().count() <= SNIPPET_CHARS {
        return collapsed;
    }
    let start = first_match_char_index(needle, &collapsed).unwrap_or(0);
    let window_start = start.saturating_sub(SNIPPET_CHARS / 4);
    let chars: Vec<char> = collapsed.chars().collect();
    let window_end = (window_start + SNIPPET_CHARS).min(chars.len());
    let slice: String = chars[window_start..window_end].iter().collect();
    let mut out = String::new();
    if window_start > 0 {
        out.push('…');
    }
    out.push_str(&slice);
    if window_end < chars.len() {
        out.push('…');
    }
    out
}

/// fzf-inspired case-insensitive subsequence score. `None` if `needle` is
/// not a subsequence of `haystack`.
pub fn score(needle: &str, haystack: &str) -> Option<i64> {
    if needle.is_empty() {
        return None;
    }
    let needle_chars: Vec<char> = needle.chars().map(normalize_char).collect();
    let hay_chars: Vec<char> = haystack.chars().collect();
    if needle_chars.len() > hay_chars.len() {
        return None;
    }

    let mut hay_index = 0;
    let mut prev_match = None::<usize>;
    let mut total = 0_i64;

    for &needle_ch in &needle_chars {
        let mut found = None;
        while hay_index < hay_chars.len() {
            let candidate = hay_chars[hay_index];
            if normalize_char(candidate) == needle_ch {
                found = Some(hay_index);
                hay_index += 1;
                break;
            }
            hay_index += 1;
        }
        let matched_at = found?;
        let mut points = 16_i64;
        if let Some(prev) = prev_match {
            let gap = matched_at - prev;
            if gap == 1 {
                points += 32;
            } else {
                points -= (gap as i64 - 1).min(8);
            }
        } else {
            // Prefer matches that start earlier in the body.
            points += (32_i64 - (matched_at as i64).min(32)).max(0);
        }
        if is_word_start(&hay_chars, matched_at) {
            points += 24;
        }
        total += points;
        prev_match = Some(matched_at);
    }

    // Prefer denser matches relative to body length.
    let density = (needle_chars.len() as i64 * 8) - (hay_chars.len() as i64 / 64);
    Some(total + density)
}

fn normalize_char(ch: char) -> char {
    ch.to_ascii_lowercase()
}

fn is_word_start(chars: &[char], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let prev = chars[index - 1];
    let curr = chars[index];
    (!prev.is_alphanumeric() && curr.is_alphanumeric())
        || (prev.is_ascii_lowercase() && curr.is_ascii_uppercase())
}

fn collapse_whitespace(body: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in body.chars() {
        if ch.is_whitespace() {
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push(' ');
        }
        gap = false;
        out.push(ch);
    }
    out
}

fn first_match_char_index(needle: &str, haystack: &str) -> Option<usize> {
    let first = needle.chars().next().map(normalize_char)?;
    haystack.chars().position(|ch| normalize_char(ch) == first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, written_at: i64, body: &str) -> DiaryEntry {
        DiaryEntry::from_parts(id, written_at, body)
    }

    #[test]
    fn normalize_query_trims_and_caps() {
        assert_eq!(normalize_query("  "), None);
        assert_eq!(normalize_query("").as_deref(), None);
        assert_eq!(normalize_query("  hello  ").as_deref(), Some("hello"));
        let long = "a".repeat(MAX_NEEDLE_CHARS + 20);
        let normalized = normalize_query(&long).unwrap();
        assert_eq!(normalized.chars().count(), MAX_NEEDLE_CHARS);
    }

    #[test]
    fn score_requires_subsequence_and_prefers_adjacency() {
        assert!(score("diary", "dear diary,").is_some());
        assert!(score("dry", "dear diary").is_some());
        assert_eq!(score("xyz", "dear diary"), None);
        let tight = score("abc", "abc").unwrap();
        let loose = score("abc", "a X b Y c").unwrap();
        assert!(
            tight > loose,
            "adjacent match should outrank gappy: {tight} vs {loose}"
        );
        let early = score("note", "note later").unwrap();
        let late = score("note", "later note").unwrap();
        assert!(early >= late);
    }

    #[test]
    fn rank_orders_by_score_then_recency() {
        let ranked = rank(
            "coffee",
            [
                entry("a", 100, "I spilled coffee everywhere"),
                entry("b", 200, "c o f f e e spaced"),
                entry("c", 300, "coffee"),
                entry("d", 400, "tea only"),
            ],
        );
        assert_eq!(
            ranked
                .iter()
                .map(|hit| hit.entry.id.as_str())
                .collect::<Vec<_>>(),
            ["c", "a", "b"]
        );
        assert!(ranked[0].score >= ranked[1].score);
    }

    #[test]
    fn page_hits_windows_like_the_transcript() {
        let ranked: Vec<RankedHit> = (0..PAGE_SIZE + 3)
            .map(|index| RankedHit {
                entry: entry(&format!("id-{index}"), index as i64, "match me"),
                score: 1,
            })
            .collect();
        assert_eq!(page_hits(&ranked, 1).len(), PAGE_SIZE);
        assert_eq!(page_hits(&ranked, 2).len(), 3);
        assert!(page_hits(&ranked, 3).is_empty());
    }

    #[test]
    fn snippet_collapses_and_windows() {
        assert_eq!(snippet("x", "  hello   world  "), "hello world");
        let long = format!("{}needle{}", "a".repeat(80), "b".repeat(80));
        let cut = snippet("needle", &long);
        assert!(cut.contains("needle"));
        assert!(cut.starts_with('…') || cut.ends_with('…'));
        assert!(cut.chars().count() <= SNIPPET_CHARS + 2);
    }
}
