# Thought comments

Every registered thought has one public, server-rendered comment tree. The
shared shell resolves the exact request path through `POSTS` and appends
`thoughts::comments::comment_section`; individual thought modules never opt in.
Thought HTML is `no-store` because comments, tombstones, and the open/closed
setting are live state.

## Identity and permissions

Any verified Google account may sign in. Hidden pages still authorize their
own grants on every request; a viewer cookie alone opens none of them.

Comment ownership uses SHA-256 of Google's stable `sub`, never email or a form
field. The public author label is a snapshot of Google's profile name; email is
not rendered in the discussion. A comment may be tombstoned by that subject or
by `ADMIN_EMAIL`. Only the admin may close or reopen a thought's comments.
Every POST repeats authentication/authorization, requires positive
same-origin evidence, checks URL-encoded content type, bounds the body, and
strictly rejects duplicate or unknown fields.

## Storage

- `thought_comments` stores one row per comment: opaque UUID id, `thought_slug`,
  optional parent id, hashed author id, optional author name/body, creation
  time, and optional deletion time.
- `thought_comment_settings` is keyed by thought slug. No row means enabled;
  this makes newly registered thoughts commentable without seed data. Its
  `create_version` is incremented by every create transaction, serializing
  otherwise-disjoint comment inserts so the limits below cannot race.
- Epoch 5 owns the `(thought_slug, created_at)` index. Table/field definitions
  remain in `src/schema.surql`; index changes remain forward migrations.

Every read and mutation is scoped by the registry-validated thought slug.
Reply creation checks the parent exists in that same thought in the same
transaction that rechecks comments are open. Parent links never change, so a
cycle cannot be created. The complete scoped set is sorted and assembled into
a preorder tree in Rust. The flat SSR preorder carries `aria-level` plus a
screen-reader nesting label; visual indentation is capped without losing the
actual semantic level.

Deletion clears `body` and `author_name` and stamps `deleted_at`; it never
removes the row or descendants. The opaque author hash remains solely for
ownership. Existing comments stay visible when a post is closed, and
authorized deletion stays available.

Comment bodies are escaped plain text, normalized to LF, and limited to 5,000
characters. Return redirects may preserve a thought's query string (including
repeated `via` keys) but are rebuilt only for that exact canonical thought
path. Renaming a registered post slug requires an explicit data migration or
its old rows will deliberately stop resolving.

Growth is bounded before the transactional create: at most 2,000 stored rows
per thought, 250 per Google subject per thought, and one create per subject
per thought every 10 seconds. Tombstones count toward both durable caps, so
delete/repost cannot grow storage. Reads ask for at most 2,001 rows and fail
closed instead of materializing an unexpectedly oversized thread. These are
safety ceilings, not pagination; increase them only alongside a bounded
rendering design.

## Routes and verification

- `POST /thoughts/{thought_slug}/comments` — new root or reply
- `POST /thoughts/{thought_slug}/comments/{comment_id}/delete` — tombstone
- `POST /thoughts/{thought_slug}/comments/settings` — admin open/close

The in-memory database tests cover cross-thought reply rejection, default-open
settings, per-thought closure, owner/admin tombstones, descendant retention,
growth/burst ceilings, tree order, forms, text validation, and safe return
targets. Run `just check` before shipping; also boot the router because
discovered-route collisions are not caught by that command.
