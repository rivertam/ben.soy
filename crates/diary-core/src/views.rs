//! The diary's shared markup: ONE definition of the transcript, bubbles,
//! compose form, and the page-JS `<template>`, rendered by the server page
//! (inside the site shell), by the service worker's offline SSR (inside
//! [`offline_page`]), and cloned by diary.js.
//!
//! Every component here is PURE — props in, markup out, zero awaits — which
//! is what keeps `Component::render`'s unconditional `+ Send` bound
//! satisfied on wasm32: a future that never suspends over `!Send` state is
//! `Send` for free. Data access stays in the hosts (the server page fn, the
//! worker's oneshot-bounced reads); nothing in this module may touch a
//! database, the clock, or app context beyond what topcoat's own expansion
//! needs.

use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::eastern;
use crate::entry::DiaryEntry;
use crate::outbox::{LocalEntry, STATE_FAILED, STATE_SYNCED};
use crate::search;
use crate::store::PAGE_SIZE;

pub const DIARY_PATH: &str = "/diary";

const META_LABEL: &str =
    "font-meta text-[0.6875rem] leading-normal tracking-[0.13em] uppercase text-muted";
const TEXTAREA: &str = "w-full min-w-0 min-h-[3rem] px-3 py-[0.65rem] text-ink bg-card \
     border border-hairline rounded-[0.2rem] font-body text-sm leading-relaxed outline-none \
     placeholder:text-muted placeholder:opacity-100 \
     hover:border-[color-mix(in_srgb,var(--color-ink2)_45%,var(--color-hairline))] \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";

/// One transcript bubble, pre-shaped by the host: the component only turns
/// this into markup.
#[derive(Clone, Debug)]
pub struct Bubble {
    pub id: String,
    pub body: String,
    pub reply_to: Option<String>,
    pub state: BubbleState,
}

#[derive(Clone, Debug)]
pub enum BubbleState {
    /// The empty shape the page JS clones and fills.
    Draft,
    /// Delivered: permalink anchor visible, note hidden.
    Synced,
    /// Queued: dashed look + "will sync" label only when the queue is
    /// actually blocked.
    Pending { blocked: bool },
    /// Rejected by the server; text kept for manual copy, discard shown.
    Failed { reason: Option<String> },
}

impl Bubble {
    pub fn synced(entry: &DiaryEntry) -> Bubble {
        Bubble {
            id: entry.id.clone(),
            body: entry.body.clone(),
            reply_to: entry.reply_to.clone(),
            state: BubbleState::Synced,
        }
    }

    /// A local row in whatever state it holds. `blocked` is the last flush
    /// report's verdict — offline SSR passes `true` (it renders precisely
    /// because the network is away).
    pub fn from_local(entry: &LocalEntry, blocked: bool) -> Bubble {
        let state = if entry.state == STATE_SYNCED {
            BubbleState::Synced
        } else if entry.state == STATE_FAILED {
            BubbleState::Failed {
                reason: entry.reason.clone(),
            }
        } else {
            BubbleState::Pending { blocked }
        };
        Bubble {
            id: entry.id.clone(),
            body: entry.body.clone(),
            reply_to: entry.reply_to.clone(),
            state,
        }
    }

    fn state_attr(&self) -> &'static str {
        match self.state {
            BubbleState::Draft => "draft",
            BubbleState::Synced => "synced",
            BubbleState::Pending { .. } => "pending",
            BubbleState::Failed { .. } => "failed",
        }
    }

    fn queued_look(&self) -> bool {
        matches!(
            self.state,
            BubbleState::Failed { .. } | BubbleState::Pending { blocked: true }
        )
    }

    fn article_class(&self) -> &'static str {
        if self.queued_look() {
            "diary-message diary-message-queued"
        } else {
            "diary-message"
        }
    }

    fn synced_href(&self) -> Option<String> {
        match self.state {
            BubbleState::Synced => Some(entry_url(&self.id)),
            _ => None,
        }
    }

    fn anchor_text(&self) -> String {
        match self.state {
            BubbleState::Synced => entry_stamp(&self.id),
            _ => String::new(),
        }
    }

    /// The note line for non-synced states: stamp plus a state label,
    /// exactly the strings diary.js writes when it toggles states live.
    fn note_text(&self) -> String {
        match &self.state {
            BubbleState::Draft | BubbleState::Synced => String::new(),
            BubbleState::Pending { blocked } => {
                let stamp = entry_stamp(&self.id);
                if *blocked {
                    format!("{stamp} · queued — will sync")
                } else {
                    stamp
                }
            }
            BubbleState::Failed { reason } => {
                let stamp = entry_stamp(&self.id);
                let reason = reason.as_deref().unwrap_or("rejected");
                format!("{stamp} · failed — {reason}")
            }
        }
    }

    fn note_hidden(&self) -> bool {
        matches!(self.state, BubbleState::Synced)
    }

    fn anchor_hidden(&self) -> bool {
        !matches!(self.state, BubbleState::Synced)
    }

    fn discard_hidden(&self) -> bool {
        !matches!(self.state, BubbleState::Failed { .. })
    }

    /// Replying is a relationship to a durable permalink, so a local
    /// draft, pending row, or failed row must never offer the control.
    fn reply_hidden(&self) -> bool {
        !matches!(self.state, BubbleState::Synced) || self.id.is_empty()
    }

    fn reply_to_hidden(&self) -> bool {
        self.reply_to.is_none()
    }

    fn reply_to_href(&self) -> Option<String> {
        self.reply_to.as_deref().map(entry_url)
    }

    fn reply_to_text(&self) -> String {
        self.reply_to
            .as_deref()
            .map(|id| format!("↳ {}", entry_stamp(id)))
            .unwrap_or_default()
    }
}

/// One bubble — the SAME markup whether it is a server-rendered synced
/// article, an offline-rendered pending row, or the template the page JS
/// clones. Every part is present in every state and toggled by `hidden`,
/// so diary.js can move a bubble between states without rebuilding it.
#[component]
pub async fn bubble(item: Bubble) -> Result {
    view! {
        <article
            class=(item.article_class())
            data-id=(item.id.as_str())
            data-state=(item.state_attr())
            data-reply-to=(item.reply_to.as_deref().unwrap_or(""))
        >
            <p
                class="diary-reply-to mb-1.5 font-meta text-[0.6875rem] text-muted"
                hidden=(item.reply_to_hidden())
            >
                <a class="quiet-link" href=(item.reply_to_href())>
                    (item.reply_to_text())
                </a>
            </p>
            <p class="diary-body leading-relaxed whitespace-pre-wrap text-ink2">(item.body.as_str())</p>
            <p class="mt-2 text-right font-meta text-[0.6875rem] text-muted">
                <span class="diary-note text-ink2" hidden=(item.note_hidden())>
                    (item.note_text())
                </span>
                <a
                    class="diary-permalink quiet-link"
                    hidden=(item.anchor_hidden())
                    href=(item.synced_href())
                >(item.anchor_text())</a>
                <button
                    type="button"
                    class="diary-reply quiet-link ml-3 cursor-pointer font-meta text-xs"
                    hidden=(item.reply_hidden())
                >"reply"</button>
                <button
                    type="button"
                    class="diary-discard quiet-link ml-3 cursor-pointer font-meta text-xs"
                    hidden=(item.discard_hidden())
                >"discard"</button>
            </p>
        </article>
    }
}

const SEARCH_INPUT: &str = "w-full min-w-0 px-3 py-1.5 text-ink bg-card \
     border border-hairline rounded-[0.2rem] font-body text-sm leading-normal outline-none \
     placeholder:text-muted placeholder:opacity-100 \
     hover:border-[color-mix(in_srgb,var(--color-ink2)_45%,var(--color-hairline))] \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";

/// One search hit for the hybrid room: stamp + truncated body, linking to
/// the permalink. Built by hosts from [`search::RankedHit`].
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub id: String,
    pub snippet: String,
}

impl SearchHit {
    pub fn from_ranked(needle: &str, hit: &search::RankedHit) -> Self {
        Self {
            id: hit.entry.id.clone(),
            snippet: search::snippet(needle, &hit.entry.body),
        }
    }
}

/// How the room's scrollback is filled. Transcript is the chat page;
/// search replaces history with ranked hits while the compose form stays.
#[derive(Clone, Debug)]
pub enum RoomMode {
    Transcript { entries: Vec<Bubble> },
    Search { query: String, hits: Vec<SearchHit> },
}

/// The transcript room: bar, search field, scrollback, history-or-hits,
/// the JS-owned queue section, the bubble template, and the compose form.
/// `notice` is the host's slot for server-only inserts (form-POST notices);
/// pass an empty view when there are none.
#[component]
#[allow(clippy::too_many_arguments)]
pub async fn diary_room(
    page_number: usize,
    last_page: usize,
    total: usize,
    store_ok: bool,
    mode: RoomMode,
    notice: View,
) -> Result {
    let template_seed = Bubble {
        id: String::new(),
        body: String::new(),
        reply_to: None,
        state: BubbleState::Draft,
    };
    let (searching, search_query, transcript_entries, search_hits) = match mode {
        RoomMode::Transcript { entries } => (false, String::new(), entries, Vec::new()),
        RoomMode::Search { query, hits } => (true, query, Vec::new(), hits),
    };
    let older_href = nav_url(&search_query, page_number + 1);
    let newer_href = nav_url(&search_query, page_number.saturating_sub(1).max(1));
    let match_label = match total {
        0 => "no matches".to_string(),
        1 => "1 match".to_string(),
        n => format!("{n} matches"),
    };
    let older_label = if searching {
        "↑ more matches"
    } else {
        "↑ older messages"
    };
    let newer_label = if searching {
        "newer matches ↓"
    } else {
        "newer messages ↓"
    };
    view! {
        <div
            class="diary-room"
            data-page=(page_number)
            data-search=(if searching { "1" } else { "" })
        >
            <div class="diary-room-bar">
                <div class="diary-room-bar-meta">
                    <span class=(META_LABEL)>"diary · just you"</span>
                    if searching {
                        <a class="quiet-link font-meta text-xs" href=(DIARY_PATH)>"clear"</a>
                    }
                </div>
                if total > PAGE_SIZE {
                    <span class="font-meta text-xs text-muted">
                        (format!("page {page_number} of {last_page}"))
                    </span>
                } else if searching {
                    <span class="font-meta text-xs text-muted">(match_label.as_str())</span>
                }
                <form class="diary-search" method="get" action=(DIARY_PATH)>
                    <label class="min-w-0 flex-1" for="diary-q">
                        <span class="sr-only">"Search diary"</span>
                        <input
                            class=(SEARCH_INPUT)
                            id="diary-q"
                            name="q"
                            type="search"
                            value=(search_query.as_str())
                            placeholder="Search…"
                            autocomplete="off"
                            maxlength=(search::MAX_NEEDLE_CHARS.to_string())
                        >
                    </label>
                    <button
                        type="submit"
                        class="oxlink shrink-0 cursor-pointer font-meta text-sm"
                    >"search"</button>
                </form>
            </div>
            <div id="diary-transcript" class="diary-transcript" tabindex="0">
                if store_ok && page_number < last_page {
                    <p class="text-center font-meta text-xs">
                        <a class="quiet-link" href=(older_href.as_str())>
                            (older_label)
                        </a>
                    </p>
                }
                (notice)
                if !store_ok {
                    <p class="text-ink2">
                        "The diary store is unreachable, so nothing can be listed "
                        "right now. Entries are safe where they are; try again in "
                        "a moment."
                    </p>
                }
                // diary.js hides this the moment any bubble renders — an
                // optimistic first message must not sit under a placeholder
                // that contradicts it.
                if store_ok && !searching && total == 0 {
                    <p id="diary-empty" class="my-auto text-center text-sm text-muted">
                        "No messages yet. Say something below."
                    </p>
                }
                if store_ok && searching && total == 0 {
                    <p class="my-auto text-center text-sm text-muted">
                        "No matching entries."
                    </p>
                }
                if searching {
                    <section class="diary-hits" aria-label="Search results">
                        for hit in search_hits.iter() {
                            search_hit(hit: hit.clone())
                        }
                    </section>
                } else {
                    <section class="diary-history" aria-label="Diary messages">
                        for item in transcript_entries.iter() {
                            bubble(item: item.clone())
                        }
                    </section>
                }
                // diary.js appends bubbles here for rows the rendered HTML
                // does not show, by cloning the template below.
                <section id="diary-queue" class="diary-queue" hidden=""></section>
                // The one definition of bubble markup the page JS may draw
                // from — cloned, never built from strings.
                <template id="diary-bubble">
                    bubble(item: template_seed)
                </template>
                if store_ok && page_number > 1 {
                    <p class="text-center font-meta text-xs">
                        <a class="quiet-link" href=(newer_href.as_str())>
                            (newer_label)
                        </a>
                    </p>
                }
            </div>
            <form
                method="post"
                action="/diary/write"
                id="diary-compose"
                class="diary-compose"
            >
                <div id="diary-replying" class="diary-replying" hidden="">
                    <span class=(META_LABEL)>
                        "Replying to "
                        <span id="diary-replying-stamp"></span>
                    </span>
                    <button
                        type="button"
                        id="diary-reply-cancel"
                        class="quiet-link cursor-pointer font-meta text-xs"
                    >"cancel"</button>
                </div>
                <input type="hidden" id="diary-reply-to" name="reply_to" value="">
                <div class="diary-compose-row">
                    <label class="min-w-0 flex-1" for="diary-body">
                        <span class="sr-only">"New diary message"</span>
                        // autofocus lands the cursor in the box on desktop;
                        // touch browsers ignore it rather than popping the
                        // keyboard. diary.js adds Enter-to-send for
                        // keyboard environments. Skip autofocus while
                        // searching so the query field keeps focus.
                        <textarea
                            class=(TEXTAREA)
                            id="diary-body"
                            name="body"
                            rows="2"
                            required=""
                            autofocus=(!searching)
                            placeholder="Message yourself…"
                        ></textarea>
                    </label>
                    <button
                        type="submit"
                        class="oxlink mb-3 shrink-0 cursor-pointer font-meta text-sm"
                    >"send ↑"</button>
                </div>
            </form>
        </div>
    }
}

#[component]
async fn search_hit(hit: SearchHit) -> Result {
    view! {
        <a class="diary-hit quiet-link" href=(entry_url(&hit.id))>
            <span class="diary-hit-stamp font-meta text-[0.6875rem] text-muted">
                (entry_stamp(&hit.id))
            </span>
            <span class="diary-hit-body text-sm leading-relaxed text-ink2">
                (hit.snippet.as_str())
            </span>
        </a>
    }
}

/// One entry's detail core — stamp, body, optional reply parent, and the
/// delete form. Hosts pass the canonical entry and add their own chrome and
/// heading around it.
#[component]
pub async fn entry_detail(entry: DiaryEntry) -> Result {
    view! {
        <section class="mt-8 max-w-prose">
            <p class="font-meta text-xs text-muted">(entry_stamp(&entry.id))</p>
            if let Some(parent) = entry.reply_to.as_ref() {
                <p class="mt-2 font-meta text-[0.6875rem] text-muted">
                    <a class="quiet-link" href=(entry_url(parent))>
                        (format!("↳ {}", entry_stamp(parent)))
                    </a>
                </p>
            }
            <p class="mt-3 leading-relaxed whitespace-pre-wrap text-ink2">(entry.body.as_str())</p>
            <form
                method="post"
                action="/diary/delete"
                class="mt-10 border-t border-hairline pt-4 text-right"
            >
                <input type="hidden" name="path" value=(entry.id.as_str())>
                <button
                    type="submit"
                    class="quiet-link cursor-pointer font-meta text-xs"
                >"delete this entry"</button>
            </form>
        </section>
    }
}

/// The minimal chrome the service worker wraps offline renders in — the
/// full site shell stays server-only by design (it awaits a grants query
/// and drags server-only context). Asset hrefs arrive as plain strings
/// (resolved server-side into the /diary-sync.js loader); no Asset
/// machinery runs on wasm.
///
/// Deliberately unthemed: the shell's theme boot script and switcher stay
/// server-side too, so an offline render comes up in the default mill and
/// oxide whatever `bens-theme` says. Cosmetic only, and only offline.
#[component]
pub async fn offline_page(
    title: &str,
    css_hrefs: Vec<String>,
    diary_js: String,
    child: View,
) -> Result {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>(title)</title>
                for href in css_hrefs.iter() {
                    <link rel="stylesheet" href=(href.as_str())>
                }
                <link rel="manifest" href="/diary.webmanifest">
                <meta name="theme-color" content="#f4f5f7">
            </head>
            <body>
                <div class="mx-auto flex min-h-screen w-full max-w-2xl flex-col px-4">
                    <header class="flex items-baseline justify-between py-4">
                        <span class="font-display font-bold">"Ben Berman"</span>
                        <span class=(META_LABEL)>"diary · offline"</span>
                    </header>
                    (child)
                </div>
                <script type="module" src=(diary_js.as_str())></script>
            </body>
        </html>
    }
}

/// `/diary/{id}` — the permalink an entry is born with.
pub fn entry_url(id: &str) -> String {
    format!("{DIARY_PATH}/{id}")
}

pub fn page_url(page_number: usize) -> String {
    nav_url("", page_number)
}

/// Transcript or search URL for the given 1-based page. Empty `q` is the
/// ordinary room; a non-empty needle keeps `q` on every page link.
pub fn nav_url(q: &str, page_number: usize) -> String {
    let mut url = DIARY_PATH.to_string();
    let mut sep = '?';
    if !q.is_empty() {
        url.push(sep);
        url.push_str("q=");
        url.push_str(&encode_query_component(q));
        sep = '&';
    }
    if page_number > 1 {
        url.push(sep);
        url.push_str("page=");
        url.push_str(&page_number.to_string());
    }
    url
}

fn encode_query_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// "Jul 27, 2026 · 2:30 PM", from the id's Eastern wall clock. Stored ids
/// always parse; anything else (drafts, synthetic failed keys) falls back
/// to the raw id.
pub fn entry_stamp(id: &str) -> String {
    match eastern::parse_public_path(id).and_then(|instant| display_parts(&instant.local)) {
        Some((date, time)) => format!("{date} · {time}"),
        None => id.to_string(),
    }
}

/// The date half alone — the entry page's heading.
pub fn entry_date(id: &str) -> String {
    match eastern::parse_public_path(id).and_then(|instant| display_parts(&instant.local)) {
        Some((date, _)) => date,
        None => id.to_string(),
    }
}

/// `YYYY-MM-DD HH:MM:SS` → ("Jul 27, 2026", "2:30 PM"). The shape is
/// guaranteed by `parse_public_path`; anything else returns `None`.
fn display_parts(local: &str) -> Option<(String, String)> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if local.len() != 19 || !local.is_ascii() {
        return None;
    }
    let month: usize = local[5..7].parse().ok()?;
    let day: u32 = local[8..10].parse().ok()?;
    let hour: u32 = local[11..13].parse().ok()?;
    if !(1..=12).contains(&month) || hour > 23 {
        return None;
    }
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let clock_hour = match hour % 12 {
        0 => 12,
        hour => hour,
    };
    Some((
        format!("{} {day}, {}", MONTHS[month - 1], &local[..4]),
        format!("{clock_hour}:{} {suffix}", &local[14..16]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_and_urls_shape_like_the_server_always_did() {
        assert_eq!(
            entry_stamp("2026-07-27T14-30-45-04-00"),
            "Jul 27, 2026 · 2:30 PM"
        );
        assert_eq!(entry_date("2026-07-27T14-30-45-04-00"), "Jul 27, 2026");
        assert_eq!(entry_stamp("garbage"), "garbage");
        assert_eq!(
            entry_url("2026-07-27T14-30-45-04-00"),
            "/diary/2026-07-27T14-30-45-04-00"
        );
        assert_eq!(page_url(1), "/diary");
        assert_eq!(page_url(3), "/diary?page=3");
        assert_eq!(nav_url("coffee", 1), "/diary?q=coffee");
        assert_eq!(nav_url("coffee", 2), "/diary?q=coffee&page=2");
        assert_eq!(nav_url("a b", 1), "/diary?q=a+b");
        assert_eq!(
            display_parts("2026-01-05 00:07:00").unwrap(),
            ("Jan 5, 2026".to_string(), "12:07 AM".to_string())
        );
        assert_eq!(
            display_parts("2026-12-31 12:00:59").unwrap(),
            ("Dec 31, 2026".to_string(), "12:00 PM".to_string())
        );
        assert_eq!(display_parts("not a stamp"), None);
    }

    #[test]
    fn bubbles_shape_their_states() {
        let synced_entry =
            DiaryEntry::from_parts("2026-07-27T14-30-45-04-00", 1_753_640_000, "hello");
        let synced = Bubble::synced(&synced_entry);
        assert_eq!(synced.state_attr(), "synced");
        assert!(!synced.queued_look());
        assert_eq!(
            synced.synced_href().as_deref(),
            Some("/diary/2026-07-27T14-30-45-04-00")
        );
        assert!(synced.note_hidden());
        assert!(synced.reply_to_hidden());
        assert!(!synced.reply_hidden());

        let mut pending_entry =
            DiaryEntry::from_parts("2026-07-27T14-30-46-04-00", 1_753_640_001, "queued");
        pending_entry.reply_to = Some("2026-07-27T14-30-45-04-00".to_string());
        let pending_quiet = Bubble::from_local(
            &LocalEntry {
                entry: pending_entry,
                state: "pending".to_string(),
                reason: None,
                enqueued_at: 5,
            },
            false,
        );
        assert_eq!(pending_quiet.state_attr(), "pending");
        assert!(
            !pending_quiet.queued_look(),
            "quiet pending has no dashed look"
        );
        assert_eq!(pending_quiet.note_text(), "Jul 27, 2026 · 2:30 PM");
        assert!(!pending_quiet.reply_to_hidden());
        assert_eq!(pending_quiet.reply_to_text(), "↳ Jul 27, 2026 · 2:30 PM");
        assert!(pending_quiet.reply_hidden());

        let pending_blocked = Bubble {
            state: BubbleState::Pending { blocked: true },
            ..pending_quiet.clone()
        };
        assert!(pending_blocked.queued_look());
        assert!(
            pending_blocked
                .note_text()
                .ends_with("· queued — will sync")
        );

        let failed = Bubble::from_local(
            &LocalEntry {
                entry: DiaryEntry::from_parts("failed-99-1", 99, "kept"),
                state: "failed".to_string(),
                reason: Some("rejected (HTTP 422)".to_string()),
                enqueued_at: 1,
            },
            true,
        );
        assert!(failed.queued_look());
        assert!(!failed.discard_hidden());
        assert!(failed.reply_hidden());
        assert_eq!(
            failed.note_text(),
            "failed-99-1 · failed — rejected (HTTP 422)"
        );
    }
}
