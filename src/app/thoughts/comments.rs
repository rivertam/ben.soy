//! Public, thought-scoped comment trees.
//!
//! Every registered thought gets this section from the shared document shell;
//! page modules do not opt in individually. Comments are plain text, grouped
//! into a tree in Rust, and stored one row at a time so unrelated writers do
//! not contend on one giant per-post record. Deletion is a tombstone: the
//! body and public author label are cleared while the row and its descendants
//! remain in place.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use benjisponge::data::{Data, Db};
use jiff::{Timestamp, tz::TimeZone};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use surrealdb::types::SurrealValue;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, Response, StatusCode, header, headers, path_param, route, to_bytes, uri,
    },
    view::{component, view},
};

use crate::{
    app::login::{MAX_AUTH_RETURN_BYTES, Viewer, viewer},
    content::{
        access::is_admin,
        posts::{POSTS, Post},
    },
    util::{is_same_origin, urlencode},
};

// A 5,000-scalar Unicode comment may expand to 60,000 bytes when every UTF-8
// byte is percent-encoded. Leave room for the return target and form keys too.
const BODY_LIMIT_BYTES: usize = 80 * 1024;
const MAX_COMMENT_CHARS: usize = 5_000;
const MAX_AUTHOR_CHARS: usize = 100;
const COMMENTS_FRAGMENT: &str = "#comments";
const MAX_RETURN_TO_BYTES: usize = MAX_AUTH_RETURN_BYTES - COMMENTS_FRAGMENT.len();
const MAX_COMMENTS_PER_THOUGHT: usize = 2_000;
const MAX_COMMENTS_PER_AUTHOR: usize = 250;
const MIN_COMMENT_INTERVAL_SECONDS: i64 = 10;
const NO_STORE: &str = "no-store";
const CLOSED_SENTINEL: &str = "thought_comments_closed";
const PARENT_SENTINEL: &str = "thought_comment_parent_missing";
const THREAD_FULL_SENTINEL: &str = "thought_comment_thread_full";
const AUTHOR_LIMIT_SENTINEL: &str = "thought_comment_author_limit";
const RATE_LIMIT_SENTINEL: &str = "thought_comment_rate_limited";
const CONFLICT_SENTINEL: &str = "Transaction conflict:";

const COMMENT_PROJECTION: &str = "\
    record::id(id) AS id, thought_slug, parent_id, author_id, author_name, \
    body, created_at, deleted_at";

#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
struct ThoughtComment {
    id: String,
    thought_slug: String,
    parent_id: Option<String>,
    /// SHA-256 of Google's stable `sub`; never rendered or accepted from a form.
    author_id: String,
    /// Snapshot of Google's profile name. Cleared when the comment is deleted.
    author_name: Option<String>,
    /// Plain text only. Cleared when the comment is deleted.
    body: Option<String>,
    created_at: i64,
    deleted_at: Option<i64>,
}

#[derive(Debug)]
struct Thread {
    comments: Vec<ThoughtComment>,
    enabled: bool,
}

#[derive(Clone, Debug)]
struct PlacedComment {
    comment: ThoughtComment,
    depth: usize,
    time_label: String,
    time_value: String,
}

impl PlacedComment {
    fn deleted(&self) -> bool {
        self.comment.deleted_at.is_some()
            || self.comment.author_name.is_none()
            || self.comment.body.is_none()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CreateError {
    Closed,
    ParentMissing,
    ThreadFull,
    AuthorLimit,
    RateLimited,
    Store(String),
}

#[derive(Clone, Copy)]
struct CommentLimits {
    max_per_thought: usize,
    max_per_author: usize,
    minimum_interval_seconds: i64,
}

const COMMENT_LIMITS: CommentLimits = CommentLimits {
    max_per_thought: MAX_COMMENTS_PER_THOUGHT,
    max_per_author: MAX_COMMENTS_PER_AUTHOR,
    minimum_interval_seconds: MIN_COMMENT_INTERVAL_SECONDS,
};

/// The discussion epilogue that the shared shell appends to every exact,
/// registered thought path. A store outage never takes the thought down.
#[component]
pub(crate) async fn comment_section(cx: &Cx, slug: &str) -> Result {
    let Some(post) = registered_post(slug) else {
        return view! {};
    };
    let current = viewer(cx);
    let loaded = load_thread(app_context::<Data>(cx), post.slug).await;
    let (comments, enabled, store_ok) = match loaded {
        Ok(thread) => (arrange(thread.comments), thread.enabled, true),
        Err(error) => {
            log_failure("load", post.slug, &error);
            (Vec::new(), false, false)
        }
    };
    let count_label = match comments.len() {
        1 => "1 comment".to_string(),
        count => format!("{count} comments"),
    };
    let return_to = render_return_to(cx, post.slug);
    let encoded_return_to = urlencode(&return_to);
    let comment_action = format!(
        "/thoughts/{}/comments?return_to={encoded_return_to}",
        post.slug
    );
    let settings_action = format!(
        "/thoughts/{}/comments/settings?return_to={encoded_return_to}",
        post.slug
    );
    let login_next = format!("{return_to}{COMMENTS_FRAGMENT}");
    let login_href = format!("/login?next={}", urlencode(&login_next));
    let notice = comment_notice(cx).and_then(notice_copy);
    let current_author_id = current.as_ref().map(|current| author_id(&current.sub));
    let current_author_name = current.as_ref().map(author_name);
    let current_is_admin = current
        .as_ref()
        .is_some_and(|current| is_admin(&current.email));

    view! {
        <section id="comments" class="comments scroll-mt-8" aria-labelledby="comments-heading">
            <div class="rail-row">
                <p class="rail-stamp rail-stamp-label">"discussion"</p>
                <div class="min-w-0">
                    <div class="comments-heading-row">
                        <div>
                            <h2 id="comments-heading" class="font-display text-3xl font-bold tracking-tight">
                                "Comments"
                            </h2>
                            if store_ok {
                                <p class="mt-1 font-meta text-xs text-muted">(count_label.as_str())</p>
                            }
                        </div>
                        if current_is_admin && store_ok {
                            <form method="post" action=(settings_action.as_str())>
                                <input type="hidden" name="enabled" value=(if enabled { "false" } else { "true" })>
                                <input type="hidden" name="return_to" value=(return_to.as_str())>
                                <button type="submit" class="comments-admin-action">
                                    if enabled { "close comments" } else { "reopen comments" }
                                </button>
                            </form>
                        }
                    </div>

                    if let Some(message) = notice {
                        <p class="comments-notice" role="status">(message)</p>
                    }

                    if !store_ok {
                        <p class="comments-state" role="status">
                            "Comments are temporarily unavailable. The thought itself is still here; try the discussion again in a moment."
                        </p>
                    } else {
                        if enabled {
                            if let (Some(current), Some(name)) = (current.as_ref(), current_author_name.as_ref()) {
                                <div class="comments-composer-wrap">
                                    <p class="comments-signed-in">
                                        "Commenting as "
                                        <span>(name.as_str())</span>
                                        if is_admin(&current.email) { " (admin)" }
                                        "."
                                    </p>
                                    comment_form(
                                        action: comment_action.as_str(),
                                        parent_id: "",
                                        return_to: return_to.as_str(),
                                        textarea_id: "new-comment",
                                        label: "Add a comment",
                                        button: "comment"
                                    )
                                </div>
                            } else {
                                <p class="comments-state">
                                    <a class="oxlink" href=(login_href.as_str())>"Sign in with Google"</a>
                                    " to join the discussion."
                                </p>
                            }
                        } else {
                            <p class="comments-state">"Comments are closed for this thought."</p>
                        }

                        if comments.is_empty() {
                            <p class="comments-empty">"No comments yet."</p>
                        } else {
                            <ol class="comment-tree" aria-label="Comment tree">
                                for placed in comments.iter() {
                                    let comment = &placed.comment;
                                    let comment_anchor = format!("comment-{}", comment.id);
                                    let delete_action = format!(
                                        "/thoughts/{}/comments/{}/delete?return_to={encoded_return_to}",
                                        post.slug,
                                        comment.id
                                    );
                                    let reply_textarea_id = format!("reply-{}", comment.id);
                                    let can_delete = !placed.deleted()
                                        && (current_author_id
                                            .as_ref()
                                            .is_some_and(|mine| mine == &comment.author_id)
                                            || current_is_admin);
                                    <li
                                        id=(comment_anchor.as_str())
                                        class="comment"
                                        role="listitem"
                                        aria-level=((placed.depth + 1).to_string())
                                        style=(format!("--comment-depth: {}", placed.depth))
                                    >
                                        <article>
                                            <header class="comment-meta">
                                                if placed.depth > 0 {
                                                    <span class="comment-depth-label">
                                                        (format!("Nested reply, level {}. ", placed.depth + 1))
                                                    </span>
                                                }
                                                if placed.deleted() {
                                                    <span class="comment-author comment-author-deleted">"[deleted]"</span>
                                                } else if let Some(name) = comment.author_name.as_deref() {
                                                    <span class="comment-author">(name)</span>
                                                }
                                                <span aria-hidden="true">"·"</span>
                                                <time datetime=(placed.time_value.as_str())>(placed.time_label.as_str())</time>
                                            </header>
                                            if placed.deleted() {
                                                <p class="comment-body comment-body-deleted">"[deleted]"</p>
                                            } else if let Some(body) = comment.body.as_deref() {
                                                <p class="comment-body">(body)</p>
                                            }
                                            if enabled && current.is_some() || can_delete {
                                                <div class="comment-actions">
                                                    if enabled && current.is_some() {
                                                        <details class="comment-action">
                                                            <summary>"reply"</summary>
                                                            <div class="comment-action-panel">
                                                                comment_form(
                                                                    action: comment_action.as_str(),
                                                                    parent_id: comment.id.as_str(),
                                                                    return_to: return_to.as_str(),
                                                                    textarea_id: reply_textarea_id.as_str(),
                                                                    label: "Reply",
                                                                    button: "reply"
                                                                )
                                                            </div>
                                                        </details>
                                                    }
                                                    if can_delete {
                                                        <details class="comment-action comment-delete-action">
                                                            <summary>"delete"</summary>
                                                            <form
                                                                method="post"
                                                                action=(delete_action.as_str())
                                                                class="comment-delete-confirm"
                                                            >
                                                                <input type="hidden" name="return_to" value=(return_to.as_str())>
                                                                <span>"This leaves a [deleted] stub so replies keep their place."</span>
                                                                <button type="submit">"confirm delete"</button>
                                                            </form>
                                                        </details>
                                                    }
                                                </div>
                                            }
                                        </article>
                                    </li>
                                }
                            </ol>
                        }
                    }
                </div>
            </div>
        </section>
    }
}

#[component]
async fn comment_form(
    action: &str,
    parent_id: &str,
    return_to: &str,
    textarea_id: &str,
    label: &str,
    button: &str,
) -> Result {
    view! {
        <form method="post" action=(action) class="comment-form">
            <input type="hidden" name="parent_id" value=(parent_id)>
            <input type="hidden" name="return_to" value=(return_to)>
            <label for=(textarea_id)>(label)</label>
            <textarea
                id=(textarea_id)
                name="body"
                rows=(if parent_id.is_empty() { "5" } else { "4" })
                maxlength=(MAX_COMMENT_CHARS.to_string())
                required=""
            ></textarea>
            <div class="comment-form-footer">
                <span>"plain text · 5,000 characters max"</span>
                <button type="submit">(button)</button>
            </div>
        </form>
    }
}

#[path_param]
struct ThoughtSlug(str);

#[path_param]
struct CommentId(str);

#[route(POST "/thoughts/{thought_slug}/comments")]
async fn create_comment(cx: &Cx, body: Body) -> Result<Response> {
    let raw_slug = path_param::<ThoughtSlug>(cx);
    let Some(post) = registered_post(raw_slug) else {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    };
    let login_return_to = query_return_to(cx, post.slug);
    let Some(current) = viewer(cx) else {
        return Ok(login_redirect(post.slug, &login_return_to));
    };
    if !is_same_origin(headers(cx)) {
        return Ok(plain(StatusCode::FORBIDDEN, "forbidden"));
    }
    let bytes = match form_bytes(cx, body).await {
        Ok(bytes) => bytes,
        Err(response) => return Ok(*response),
    };
    let Some(form) = parse_comment_form(&bytes) else {
        return Ok(plain(StatusCode::BAD_REQUEST, "bad form"));
    };
    let return_to = safe_return_to(post.slug, &form.return_to);
    let Some(comment_body) = normalize_comment(&form.body) else {
        return Ok(back(post.slug, &return_to, "invalid", None));
    };
    let parent_id = if form.parent_id.is_empty() {
        None
    } else if is_comment_id(&form.parent_id) {
        Some(form.parent_id)
    } else {
        return Ok(back(post.slug, &return_to, "invalid", None));
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let row = ThoughtComment {
        id: id.clone(),
        thought_slug: post.slug.to_string(),
        parent_id,
        author_id: author_id(&current.sub),
        author_name: Some(author_name(&current)),
        body: Some(comment_body),
        created_at: epoch_seconds(),
        deleted_at: None,
    };
    let db = match app_context::<Data>(cx).db().await {
        Ok(db) => db,
        Err(error) => {
            log_failure("create/connect", post.slug, &error.to_string());
            return Ok(back(post.slug, &return_to, "unavailable", None));
        }
    };
    match insert_comment(&db, &row).await {
        Ok(()) => Ok(back(post.slug, &return_to, "posted", Some(&id))),
        Err(CreateError::Closed) => Ok(back(post.slug, &return_to, "closed", None)),
        Err(CreateError::ParentMissing) => Ok(back(post.slug, &return_to, "invalid", None)),
        Err(CreateError::ThreadFull) => Ok(back(post.slug, &return_to, "full", None)),
        Err(CreateError::AuthorLimit) => Ok(back(post.slug, &return_to, "author_limit", None)),
        Err(CreateError::RateLimited) => Ok(back(post.slug, &return_to, "rate_limited", None)),
        Err(CreateError::Store(error)) => {
            log_failure("create", post.slug, &error);
            Ok(back(post.slug, &return_to, "unavailable", None))
        }
    }
}

#[route(POST "/thoughts/{thought_slug}/comments/{comment_id}/delete")]
async fn delete_comment(cx: &Cx, body: Body) -> Result<Response> {
    let raw_slug = path_param::<ThoughtSlug>(cx);
    let raw_id = path_param::<CommentId>(cx);
    let Some(post) = registered_post(raw_slug) else {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    };
    if !is_comment_id(raw_id) {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    }
    let login_return_to = query_return_to(cx, post.slug);
    let Some(current) = viewer(cx) else {
        return Ok(login_redirect(post.slug, &login_return_to));
    };
    if !is_same_origin(headers(cx)) {
        return Ok(plain(StatusCode::FORBIDDEN, "forbidden"));
    }
    let bytes = match form_bytes(cx, body).await {
        Ok(bytes) => bytes,
        Err(response) => return Ok(*response),
    };
    let Some(form) = parse_delete_form(&bytes) else {
        return Ok(plain(StatusCode::BAD_REQUEST, "bad form"));
    };
    let return_to = safe_return_to(post.slug, &form.return_to);
    let db = match app_context::<Data>(cx).db().await {
        Ok(db) => db,
        Err(error) => {
            log_failure("delete/connect", post.slug, &error.to_string());
            return Ok(back(post.slug, &return_to, "unavailable", None));
        }
    };
    match tombstone_comment(
        &db,
        post.slug,
        raw_id,
        &author_id(&current.sub),
        is_admin(&current.email),
        epoch_seconds(),
    )
    .await
    {
        Ok(true) => Ok(back(post.slug, &return_to, "deleted", None)),
        Ok(false) => Ok(plain(StatusCode::NOT_FOUND, "not found")),
        Err(error) => {
            log_failure("delete", post.slug, &error);
            Ok(back(post.slug, &return_to, "unavailable", None))
        }
    }
}

#[route(POST "/thoughts/{thought_slug}/comments/settings")]
async fn update_settings(cx: &Cx, body: Body) -> Result<Response> {
    let raw_slug = path_param::<ThoughtSlug>(cx);
    let Some(post) = registered_post(raw_slug) else {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    };
    let login_return_to = query_return_to(cx, post.slug);
    let Some(current) = viewer(cx) else {
        return Ok(login_redirect(post.slug, &login_return_to));
    };
    if !is_admin(&current.email) {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    }
    if !is_same_origin(headers(cx)) {
        return Ok(plain(StatusCode::FORBIDDEN, "forbidden"));
    }
    let bytes = match form_bytes(cx, body).await {
        Ok(bytes) => bytes,
        Err(response) => return Ok(*response),
    };
    let Some(form) = parse_settings_form(&bytes) else {
        return Ok(plain(StatusCode::BAD_REQUEST, "bad form"));
    };
    let return_to = safe_return_to(post.slug, &form.return_to);
    let db = match app_context::<Data>(cx).db().await {
        Ok(db) => db,
        Err(error) => {
            log_failure("settings/connect", post.slug, &error.to_string());
            return Ok(back(post.slug, &return_to, "unavailable", None));
        }
    };
    match set_comments_enabled(&db, post.slug, form.enabled, epoch_seconds()).await {
        Ok(()) => Ok(back(
            post.slug,
            &return_to,
            if form.enabled { "opened" } else { "closed" },
            None,
        )),
        Err(error) => {
            log_failure("settings", post.slug, &error);
            Ok(back(post.slug, &return_to, "unavailable", None))
        }
    }
}

async fn load_thread(data: &Data, slug: &str) -> std::result::Result<Thread, String> {
    let db = data.db().await.map_err(|error| error.to_string())?;
    load_thread_from_db(&db, slug).await
}

async fn load_thread_from_db(db: &Db, slug: &str) -> std::result::Result<Thread, String> {
    let mut response = db
        .query(format!(
            "SELECT {COMMENT_PROJECTION} FROM thought_comments \
                 WHERE thought_slug = $thought_slug \
                 ORDER BY created_at ASC LIMIT $load_limit; \
             SELECT VALUE comments_enabled \
                 FROM type::record('thought_comment_settings', $thought_slug);"
        ))
        .bind(("thought_slug", slug.to_string()))
        .bind((
            "load_limit",
            i64::try_from(MAX_COMMENTS_PER_THOUGHT + 1).expect("comment limit fits i64"),
        ))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let comments: Vec<ThoughtComment> = response.take(0).map_err(|error| error.to_string())?;
    if comments.len() > MAX_COMMENTS_PER_THOUGHT {
        return Err("thought comment safety limit exceeded".to_string());
    }
    let settings: Vec<bool> = response.take(1).map_err(|error| error.to_string())?;
    Ok(Thread {
        comments,
        enabled: settings.into_iter().next().unwrap_or(true),
    })
}

async fn insert_comment(db: &Db, comment: &ThoughtComment) -> std::result::Result<(), CreateError> {
    insert_comment_with_limits(db, comment, COMMENT_LIMITS).await
}

async fn insert_comment_with_limits(
    db: &Db,
    comment: &ThoughtComment,
    limits: CommentLimits,
) -> std::result::Result<(), CreateError> {
    for attempt in 0..3 {
        let result = db
            .query(
                "BEGIN TRANSACTION;
                 LET $settings = SELECT VALUE comments_enabled
                     FROM type::record('thought_comment_settings', $thought_slug);
                 IF array::len($settings) > 0 AND array::first($settings) = false {
                     THROW 'thought_comments_closed';
                 };
                 IF $parent_id IS NOT NONE {
                     LET $parent_thought = SELECT VALUE thought_slug
                         FROM type::record('thought_comments', $parent_id);
                     IF array::len($parent_thought) != 1
                         OR array::first($parent_thought) != $thought_slug {
                         THROW 'thought_comment_parent_missing';
                     };
                 };
                 UPSERT ONLY type::record('thought_comment_settings', $thought_slug)
                     SET thought_slug = $thought_slug,
                         comments_enabled = comments_enabled ?? true,
                         updated_at = updated_at ?? $created_at,
                         create_version = (create_version ?? 0) + 1
                     RETURN NONE;
                 LET $thought_comments = SELECT VALUE record::id(id)
                     FROM thought_comments
                     WHERE thought_slug = $thought_slug
                     LIMIT $max_thought_comments;
                 IF array::len($thought_comments) >= $max_thought_comments {
                     THROW 'thought_comment_thread_full';
                 };
                 LET $author_comments = SELECT VALUE created_at
                     FROM thought_comments
                     WHERE thought_slug = $thought_slug AND author_id = $author_id
                     ORDER BY created_at DESC
                     LIMIT $max_author_comments;
                 IF array::len($author_comments) >= $max_author_comments {
                     THROW 'thought_comment_author_limit';
                 };
                 IF array::len($author_comments) > 0
                     AND array::first($author_comments) > $created_at - $minimum_interval {
                     THROW 'thought_comment_rate_limited';
                 };
                 CREATE ONLY type::record('thought_comments', $comment.id)
                     CONTENT $comment RETURN NONE;
                 COMMIT TRANSACTION;",
            )
            .bind(("thought_slug", comment.thought_slug.clone()))
            .bind(("parent_id", comment.parent_id.clone()))
            .bind(("author_id", comment.author_id.clone()))
            .bind(("created_at", comment.created_at))
            .bind((
                "max_thought_comments",
                i64::try_from(limits.max_per_thought).expect("comment limit fits i64"),
            ))
            .bind((
                "max_author_comments",
                i64::try_from(limits.max_per_author).expect("author comment limit fits i64"),
            ))
            .bind(("minimum_interval", limits.minimum_interval_seconds))
            .bind(("comment", comment.clone()))
            .await
            .map_err(|error| error.to_string())
            .and_then(|mut response| {
                // A transaction-level THROW also marks later transaction
                // slots as failed. `check()` may surface either one first,
                // so collect every statement error to retain our deliberate
                // closed/parent outcome, then still check the remainder.
                let mut errors: Vec<_> = response.take_errors().into_iter().collect();
                errors.sort_unstable_by_key(|(index, _)| *index);
                let remainder = response.check().map_err(|error| error.to_string());
                if errors.is_empty() {
                    remainder.map(|_| ())
                } else {
                    let mut messages: Vec<String> = errors
                        .into_iter()
                        .map(|(_, error)| error.to_string())
                        .collect();
                    if let Err(error) = remainder {
                        messages.push(error);
                    }
                    Err(messages.join("; "))
                }
            });
        match result {
            Ok(_) => return Ok(()),
            Err(error) if error.contains(CONFLICT_SENTINEL) && attempt < 2 => {
                tokio::task::yield_now().await;
            }
            Err(error) if error.contains(CLOSED_SENTINEL) => return Err(CreateError::Closed),
            Err(error) if error.contains(PARENT_SENTINEL) => {
                return Err(CreateError::ParentMissing);
            }
            Err(error) if error.contains(THREAD_FULL_SENTINEL) => {
                return Err(CreateError::ThreadFull);
            }
            Err(error) if error.contains(AUTHOR_LIMIT_SENTINEL) => {
                return Err(CreateError::AuthorLimit);
            }
            Err(error) if error.contains(RATE_LIMIT_SENTINEL) => {
                return Err(CreateError::RateLimited);
            }
            Err(error) => return Err(CreateError::Store(error)),
        }
    }
    unreachable!("the bounded retry loop always returns")
}

async fn tombstone_comment(
    db: &Db,
    slug: &str,
    id: &str,
    author_id: &str,
    admin: bool,
    deleted_at: i64,
) -> std::result::Result<bool, String> {
    for attempt in 0..3 {
        let result = async {
            let mut response = db
                .query(
                    "UPDATE type::record('thought_comments', $id)
                         SET author_name = NONE, body = NONE, deleted_at = $deleted_at
                         WHERE thought_slug = $thought_slug
                             AND deleted_at IS NONE
                             AND ($admin = true OR author_id = $author_id)
                         RETURN VALUE record::id(id);",
                )
                .bind(("id", id.to_string()))
                .bind(("thought_slug", slug.to_string()))
                .bind(("author_id", author_id.to_string()))
                .bind(("admin", admin))
                .bind(("deleted_at", deleted_at))
                .await
                .map_err(|error| error.to_string())?
                .check()
                .map_err(|error| error.to_string())?;
            let updated: Vec<String> = response.take(0).map_err(|error| error.to_string())?;
            Ok::<bool, String>(updated.into_iter().next().as_deref() == Some(id))
        }
        .await;
        match result {
            Err(error) if error.contains(CONFLICT_SENTINEL) && attempt < 2 => {
                tokio::task::yield_now().await;
            }
            result => return result,
        }
    }
    unreachable!("the bounded retry loop always returns")
}

async fn set_comments_enabled(
    db: &Db,
    slug: &str,
    enabled: bool,
    updated_at: i64,
) -> std::result::Result<(), String> {
    for attempt in 0..3 {
        let result = db
            .query(
                "UPSERT ONLY type::record('thought_comment_settings', $thought_slug)
                     SET thought_slug = $thought_slug,
                         comments_enabled = $enabled,
                         updated_at = $updated_at
                     RETURN NONE;",
            )
            .bind(("thought_slug", slug.to_string()))
            .bind(("enabled", enabled))
            .bind(("updated_at", updated_at))
            .await
            .map_err(|error| error.to_string())
            .and_then(|response| response.check().map_err(|error| error.to_string()));
        match result {
            Ok(_) => return Ok(()),
            Err(error) if error.contains(CONFLICT_SENTINEL) && attempt < 2 => {
                tokio::task::yield_now().await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the bounded retry loop always returns")
}

/// Stable pre-order with chronological/id sibling order. Missing parents are
/// treated as roots; a corrupt cycle is still rendered once rather than
/// recursing forever or hiding rows.
fn arrange(mut comments: Vec<ThoughtComment>) -> Vec<PlacedComment> {
    comments.sort_unstable_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let indexes: HashMap<&str, usize> = comments
        .iter()
        .enumerate()
        .map(|(index, comment)| (comment.id.as_str(), index))
        .collect();
    let mut roots = Vec::new();
    let mut children: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, comment) in comments.iter().enumerate() {
        match comment.parent_id.as_deref() {
            Some(parent) if parent != comment.id && indexes.contains_key(parent) => {
                children.entry(parent.to_string()).or_default().push(index);
            }
            _ => roots.push(index),
        }
    }

    let mut ordered = Vec::with_capacity(comments.len());
    let mut visited = HashSet::with_capacity(comments.len());
    append_forest(
        &comments,
        &children,
        roots.into_iter().map(|index| (index, 0)),
        &mut visited,
        &mut ordered,
    );
    // Only malformed cycles can remain. Give each first unseen member a root
    // position; visited fencing makes the walk finite.
    for index in 0..comments.len() {
        if !visited.contains(&index) {
            append_forest(
                &comments,
                &children,
                [(index, 0)],
                &mut visited,
                &mut ordered,
            );
        }
    }
    ordered
}

fn append_forest(
    comments: &[ThoughtComment],
    children: &HashMap<String, Vec<usize>>,
    roots: impl IntoIterator<Item = (usize, usize)>,
    visited: &mut HashSet<usize>,
    ordered: &mut Vec<PlacedComment>,
) {
    let roots: Vec<_> = roots.into_iter().collect();
    let mut stack: Vec<_> = roots.into_iter().rev().collect();
    while let Some((index, depth)) = stack.pop() {
        if !visited.insert(index) {
            continue;
        }
        let comment = comments[index].clone();
        let (time_label, time_value) = format_comment_time(comment.created_at);
        ordered.push(PlacedComment {
            comment: comment.clone(),
            depth,
            time_label,
            time_value,
        });
        if let Some(nested) = children.get(&comment.id) {
            stack.extend(nested.iter().rev().map(|child| (*child, depth + 1)));
        }
    }
}

fn eastern_time_zone() -> &'static TimeZone {
    static EASTERN: OnceLock<TimeZone> = OnceLock::new();
    EASTERN.get_or_init(|| TimeZone::get("America/New_York").expect("bundled tzdb has New York"))
}

fn format_comment_time(seconds: i64) -> (String, String) {
    let Ok(timestamp) = Timestamp::from_second(seconds) else {
        return ("unknown time".to_string(), String::new());
    };
    let label = timestamp
        .to_zoned(eastern_time_zone().clone())
        .strftime("%b %-d, %Y · %-I:%M %p %Z")
        .to_string();
    (label, timestamp.to_string())
}

fn registered_post(slug: &str) -> Option<&'static Post> {
    POSTS.iter().copied().find(|post| post.slug == slug)
}

fn author_id(subject: &str) -> String {
    Sha256::digest(subject.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn author_name(current: &Viewer) -> String {
    current
        .name
        .as_deref()
        .and_then(normalize_author_name)
        .unwrap_or_else(|| "Google user".to_string())
}

fn normalize_author_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let count = name.chars().count();
    (count > 0
        && count <= MAX_AUTHOR_CHARS
        && !name
            .chars()
            .any(|character| character.is_control() || character == '\u{fffd}'))
    .then(|| name.to_string())
}

fn normalize_comment(raw: &str) -> Option<String> {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let body = normalized.trim();
    let count = body.chars().count();
    (count > 0
        && count <= MAX_COMMENT_CHARS
        && !body.chars().any(|character| {
            (character.is_control() && character != '\n' && character != '\t')
                || character == '\u{fffd}'
        }))
    .then(|| body.to_string())
}

fn is_comment_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

struct CommentForm {
    body: String,
    parent_id: String,
    return_to: String,
}

fn parse_comment_form(body: &[u8]) -> Option<CommentForm> {
    let mut comment = None;
    let mut parent_id = None;
    let mut return_to = None;
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "body" if comment.is_none() => comment = Some(value.into_owned()),
            "parent_id" if parent_id.is_none() => parent_id = Some(value.into_owned()),
            "return_to" if return_to.is_none() => return_to = Some(value.into_owned()),
            _ => return None,
        }
    }
    Some(CommentForm {
        body: comment?,
        parent_id: parent_id?,
        return_to: return_to?,
    })
}

struct DeleteForm {
    return_to: String,
}

fn parse_delete_form(body: &[u8]) -> Option<DeleteForm> {
    let mut return_to = None;
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "return_to" if return_to.is_none() => return_to = Some(value.into_owned()),
            _ => return None,
        }
    }
    Some(DeleteForm {
        return_to: return_to?,
    })
}

struct SettingsForm {
    enabled: bool,
    return_to: String,
}

fn parse_settings_form(body: &[u8]) -> Option<SettingsForm> {
    let mut enabled = None;
    let mut return_to = None;
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "enabled" if enabled.is_none() => {
                enabled = match value.as_ref() {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => return None,
                };
            }
            "return_to" if return_to.is_none() => return_to = Some(value.into_owned()),
            _ => return None,
        }
    }
    Some(SettingsForm {
        enabled: enabled?,
        return_to: return_to?,
    })
}

async fn form_bytes(cx: &Cx, body: Body) -> std::result::Result<Vec<u8>, Box<Response>> {
    if !is_form_content_type(headers(cx)) {
        return Err(Box::new(plain(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        )));
    }
    if let Some(length) = headers(cx).get(header::CONTENT_LENGTH) {
        let length = length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| Box::new(plain(StatusCode::BAD_REQUEST, "bad Content-Length")))?;
        if length > BODY_LIMIT_BYTES {
            return Err(Box::new(plain(
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            )));
        }
    }
    to_bytes(body, BODY_LIMIT_BYTES)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|_| Box::new(plain(StatusCode::PAYLOAD_TOO_LARGE, "form is too large")))
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

fn render_return_to(cx: &Cx, slug: &str) -> String {
    safe_return_to(
        slug,
        uri(cx)
            .path_and_query()
            .map_or(uri(cx).path(), |value| value.as_str()),
    )
}

/// Accept only the exact registered thought path, then decode/re-encode its
/// query while dropping our status notice. Repeated keys (notably planes'
/// `via`) retain their order.
fn safe_return_to(slug: &str, raw: &str) -> String {
    let base = format!("/thoughts/{slug}");
    if raw.len() > MAX_RETURN_TO_BYTES
        || raw.contains('#')
        || !raw.bytes().all(|byte| (0x21..0x7f).contains(&byte))
    {
        return base;
    }
    let Some(rest) = raw.strip_prefix(&base) else {
        return base;
    };
    if rest.is_empty() {
        return base;
    }
    let Some(query) = rest.strip_prefix('?') else {
        return base;
    };
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if key != "comment_notice" {
            serializer.append_pair(&key, &value);
        }
    }
    let query = serializer.finish();
    let target = if query.is_empty() {
        base
    } else {
        format!("{base}?{query}")
    };
    if target.len() <= MAX_RETURN_TO_BYTES {
        target
    } else {
        format!("/thoughts/{slug}")
    }
}

fn comment_notice(cx: &Cx) -> Option<String> {
    form_urlencoded::parse(uri(cx).query()?.as_bytes())
        .find(|(key, _)| key == "comment_notice")
        .map(|(_, value)| value.into_owned())
}

/// Forms repeat the canonical return target in their action query so an
/// expired viewer cookie can preserve a query-driven thought across login
/// without reading or retaining the submitted comment body.
fn query_return_to(cx: &Cx, slug: &str) -> String {
    let candidate = uri(cx).query().and_then(|query| {
        let mut matches = form_urlencoded::parse(query.as_bytes())
            .filter(|(key, _)| key == "return_to")
            .map(|(_, value)| value.into_owned());
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    });
    candidate.map_or_else(
        || format!("/thoughts/{slug}"),
        |candidate| safe_return_to(slug, &candidate),
    )
}

fn notice_copy(code: String) -> Option<&'static str> {
    match code.as_str() {
        "posted" => Some("Comment posted."),
        "deleted" => Some("Comment deleted; its replies remain in place."),
        "opened" => Some("Comments reopened."),
        "closed" => Some("Comments closed."),
        "invalid" => Some("That comment did not validate; nothing was posted."),
        "full" => Some("This discussion has reached its 2,000-comment safety limit."),
        "author_limit" => Some("This account has reached its per-thought comment limit."),
        "rate_limited" => Some("Please wait a few seconds before commenting again."),
        "unavailable" => Some("The comment store did not answer; nothing changed."),
        _ => None,
    }
}

fn login_redirect(slug: &str, return_to: &str) -> Response {
    let next = format!("{}{COMMENTS_FRAGMENT}", safe_return_to(slug, return_to));
    see_other(&format!("/login?next={}", urlencode(&next)))
}

fn back(slug: &str, return_to: &str, notice: &'static str, comment_id: Option<&str>) -> Response {
    let target = safe_return_to(slug, return_to);
    let separator = if target.contains('?') { '&' } else { '?' };
    let anchor = comment_id.map_or_else(|| "#comments".to_string(), |id| format!("#comment-{id}"));
    see_other(&format!(
        "{target}{separator}comment_notice={notice}{anchor}"
    ))
}

fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, NO_STORE)
        .body(Body::from("see other"))
        .expect("validated comment redirect is a valid header")
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(message))
        .expect("static comment response headers")
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn log_failure(step: &str, slug: &str, error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "thought comments failed",
            "step": step,
            "thought_slug": slug,
            "error": error,
        })
    );
}

#[cfg(test)]
mod tests {
    use surrealdb::engine::any;

    use super::*;

    async fn db() -> Db {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        db.query(include_str!("../../schema.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        db
    }

    fn row(id: &str, slug: &str, parent_id: Option<&str>, created_at: i64) -> ThoughtComment {
        ThoughtComment {
            id: id.to_string(),
            thought_slug: slug.to_string(),
            parent_id: parent_id.map(str::to_string),
            author_id: author_id("google-subject"),
            author_name: Some("Alice".to_string()),
            body: Some(format!("body {id}")),
            created_at,
            deleted_at: None,
        }
    }

    #[test]
    fn comment_text_is_plain_bounded_and_normalized() {
        assert_eq!(
            normalize_comment("  hello\r\nworld  ").as_deref(),
            Some("hello\nworld")
        );
        assert_eq!(normalize_comment("emoji 🧽").as_deref(), Some("emoji 🧽"));
        assert!(normalize_comment("").is_none());
        assert!(normalize_comment(" \n ").is_none());
        assert!(normalize_comment("bad\0text").is_none());
        assert!(normalize_comment(&"x".repeat(MAX_COMMENT_CHARS + 1)).is_none());

        let full_unicode_body = "🧽".repeat(MAX_COMMENT_CHARS);
        assert!(normalize_comment(&full_unicode_body).is_some());
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        serializer.append_pair("body", &full_unicode_body);
        serializer.append_pair("parent_id", "");
        serializer.append_pair(
            "return_to",
            &format!(
                "/thoughts/pesky-code?{}",
                "&".repeat(MAX_RETURN_TO_BYTES - "/thoughts/pesky-code?".len())
            ),
        );
        let encoded = serializer.finish();
        assert!(
            encoded.len() <= BODY_LIMIT_BYTES,
            "the advertised Unicode/comment and return-target maxima must fit the form body"
        );
    }

    #[test]
    fn forms_are_strict_about_fields_and_duplicates() {
        let parsed =
            parse_comment_form(b"body=hello&parent_id=&return_to=%2Fthoughts%2Fpesky-code")
                .unwrap();
        assert_eq!(parsed.body, "hello");
        assert!(parsed.parent_id.is_empty());
        assert!(parse_comment_form(b"body=hello&parent_id=").is_none());
        assert!(
            parse_comment_form(
                b"body=hello&body=again&parent_id=&return_to=%2Fthoughts%2Fpesky-code"
            )
            .is_none()
        );
        assert!(parse_delete_form(b"return_to=%2Fx&extra=1").is_none());
        assert!(parse_settings_form(b"enabled=maybe&return_to=%2Fx").is_none());
        assert!(!is_comment_id("ABCDEF0123456789abcdef0123456789"));
        assert!(is_comment_id("abcdef0123456789abcdef0123456789"));
    }

    #[test]
    fn return_targets_stay_on_the_thought_and_keep_repeated_query_keys() {
        assert_eq!(
            safe_return_to(
                "how-bad-are-planes",
                "/thoughts/how-bad-are-planes?from=JFK&via=KEF&via=AMS&comment_notice=posted"
            ),
            "/thoughts/how-bad-are-planes?from=JFK&via=KEF&via=AMS"
        );
        assert_eq!(
            safe_return_to("pesky-code", "https://evil.example"),
            "/thoughts/pesky-code"
        );
        assert_eq!(
            safe_return_to("pesky-code", "/thoughts/pesky-code-evil?x=1"),
            "/thoughts/pesky-code"
        );
        assert_eq!(
            safe_return_to("pesky-code", "/thoughts/pesky-code#bad"),
            "/thoughts/pesky-code"
        );

        let prefix = "/thoughts/pesky-code?q=";
        let boundary = format!("{prefix}{}", "a".repeat(MAX_RETURN_TO_BYTES - prefix.len()));
        assert_eq!(safe_return_to("pesky-code", &boundary), boundary);
        assert_eq!(
            boundary.len() + COMMENTS_FRAGMENT.len(),
            MAX_AUTH_RETURN_BYTES
        );
        assert_eq!(
            safe_return_to("pesky-code", &format!("{boundary}a")),
            "/thoughts/pesky-code"
        );
        let expanding = format!("{prefix}{}", "/".repeat(MAX_RETURN_TO_BYTES - prefix.len()));
        assert_eq!(expanding.len(), MAX_RETURN_TO_BYTES);
        assert_eq!(
            safe_return_to("pesky-code", &expanding),
            "/thoughts/pesky-code",
            "canonical percent-encoding must not expand past OAuth's budget"
        );
    }

    #[test]
    fn tree_is_preordered_stable_and_keeps_orphans_visible() {
        let root_a = "00000000000000000000000000000001";
        let root_b = "00000000000000000000000000000002";
        let child = "00000000000000000000000000000003";
        let grandchild = "00000000000000000000000000000004";
        let orphan = "00000000000000000000000000000005";
        let placed = arrange(vec![
            row(grandchild, "post", Some(child), 4),
            row(root_b, "post", None, 2),
            row(orphan, "post", Some("ffffffffffffffffffffffffffffffff"), 5),
            row(child, "post", Some(root_a), 3),
            row(root_a, "post", None, 1),
        ]);
        assert_eq!(
            placed
                .iter()
                .map(|placed| (placed.comment.id.as_str(), placed.depth))
                .collect::<Vec<_>>(),
            [
                (root_a, 0),
                (child, 1),
                (grandchild, 2),
                (root_b, 0),
                (orphan, 0),
            ]
        );
    }

    #[tokio::test]
    async fn database_scopes_replies_and_closed_state_to_the_thought() {
        let db = db().await;
        let root_id = "00000000000000000000000000000001";
        let root = row(root_id, "pesky-code", None, 1);
        insert_comment(&db, &root).await.unwrap();

        let reply = row(
            "00000000000000000000000000000002",
            "pesky-code",
            Some(root_id),
            11,
        );
        insert_comment(&db, &reply).await.unwrap();

        let cross_thought = row(
            "00000000000000000000000000000003",
            "same-age-as-my-dog",
            Some(root_id),
            3,
        );
        assert_eq!(
            insert_comment(&db, &cross_thought).await,
            Err(CreateError::ParentMissing)
        );

        set_comments_enabled(&db, "pesky-code", false, 4)
            .await
            .unwrap();
        let blocked = row("00000000000000000000000000000004", "pesky-code", None, 5);
        assert_eq!(
            insert_comment(&db, &blocked).await,
            Err(CreateError::Closed)
        );

        let thread = load_thread_from_db(&db, "pesky-code").await.unwrap();
        assert!(!thread.enabled);
        assert_eq!(thread.comments.len(), 2);
        assert!(
            load_thread_from_db(&db, "same-age-as-my-dog")
                .await
                .unwrap()
                .enabled
        );
    }

    #[tokio::test]
    async fn tombstones_require_owner_or_admin_and_retain_descendants() {
        let db = db().await;
        let root_id = "10000000000000000000000000000001";
        let root = row(root_id, "pesky-code", None, 1);
        insert_comment(&db, &root).await.unwrap();
        let reply = row(
            "10000000000000000000000000000002",
            "pesky-code",
            Some(root_id),
            11,
        );
        insert_comment(&db, &reply).await.unwrap();

        assert!(
            !tombstone_comment(
                &db,
                "pesky-code",
                root_id,
                &author_id("someone-else"),
                false,
                3,
            )
            .await
            .unwrap()
        );
        assert!(
            !tombstone_comment(
                &db,
                "same-age-as-my-dog",
                root_id,
                &author_id("google-subject"),
                false,
                3,
            )
            .await
            .unwrap(),
            "ownership never crosses a thought scope"
        );
        assert!(
            tombstone_comment(
                &db,
                "pesky-code",
                root_id,
                &author_id("google-subject"),
                false,
                3,
            )
            .await
            .unwrap()
        );

        let thread = load_thread_from_db(&db, "pesky-code").await.unwrap();
        let placed = arrange(thread.comments);
        assert_eq!(placed.len(), 2);
        assert!(placed[0].deleted());
        assert_eq!(placed[1].depth, 1);
        assert_eq!(placed[1].comment.parent_id.as_deref(), Some(root_id));

        let admin_target_id = "10000000000000000000000000000003";
        let mut admin_target = row(admin_target_id, "pesky-code", None, 4);
        admin_target.author_id = author_id("different-author");
        insert_comment(&db, &admin_target).await.unwrap();
        set_comments_enabled(&db, "pesky-code", false, 5)
            .await
            .unwrap();
        assert!(
            tombstone_comment(
                &db,
                "pesky-code",
                admin_target_id,
                &author_id("admin-subject"),
                true,
                6,
            )
            .await
            .unwrap(),
            "the admin may tombstone another author's comment after closure"
        );
    }

    #[tokio::test]
    async fn database_bounds_bursts_and_persistent_thread_growth() {
        let db = db().await;
        let limits = CommentLimits {
            max_per_thought: 3,
            max_per_author: 2,
            minimum_interval_seconds: 10,
        };
        let first = row("20000000000000000000000000000001", "pesky-code", None, 100);
        insert_comment_with_limits(&db, &first, limits)
            .await
            .unwrap();

        let too_fast = row("20000000000000000000000000000002", "pesky-code", None, 109);
        assert_eq!(
            insert_comment_with_limits(&db, &too_fast, limits).await,
            Err(CreateError::RateLimited)
        );

        let second = row("20000000000000000000000000000003", "pesky-code", None, 110);
        insert_comment_with_limits(&db, &second, limits)
            .await
            .unwrap();
        let author_overflow = row("20000000000000000000000000000004", "pesky-code", None, 120);
        assert_eq!(
            insert_comment_with_limits(&db, &author_overflow, limits).await,
            Err(CreateError::AuthorLimit)
        );

        let mut third = row("20000000000000000000000000000005", "pesky-code", None, 120);
        third.author_id = author_id("second-google-account");
        insert_comment_with_limits(&db, &third, limits)
            .await
            .unwrap();
        let mut thread_overflow = row("20000000000000000000000000000006", "pesky-code", None, 130);
        thread_overflow.author_id = author_id("third-google-account");
        assert_eq!(
            insert_comment_with_limits(&db, &thread_overflow, limits).await,
            Err(CreateError::ThreadFull)
        );
    }

    #[tokio::test]
    async fn concurrent_creates_share_a_guard_and_cannot_cross_the_thread_cap() {
        let db = db().await;
        let limits = CommentLimits {
            max_per_thought: 1,
            max_per_author: 5,
            minimum_interval_seconds: 0,
        };
        let left = row("30000000000000000000000000000001", "pesky-code", None, 100);
        let mut right = row("30000000000000000000000000000002", "pesky-code", None, 100);
        right.author_id = author_id("another-google-account");

        let (left_result, right_result) = tokio::join!(
            insert_comment_with_limits(&db, &left, limits),
            insert_comment_with_limits(&db, &right, limits),
        );
        assert!(
            matches!(
                (&left_result, &right_result),
                (Ok(()), Err(CreateError::ThreadFull)) | (Err(CreateError::ThreadFull), Ok(()))
            ),
            "one create must win and one must observe the full thread: {left_result:?}, {right_result:?}"
        );

        let thread = load_thread_from_db(&db, "pesky-code").await.unwrap();
        assert_eq!(thread.comments.len(), 1);
        let mut response = db
            .query(
                "SELECT VALUE create_version
                 FROM type::record('thought_comment_settings', 'pesky-code');",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let versions: Vec<i64> = response.take(0).unwrap();
        assert_eq!(versions, [1]);
    }
}
