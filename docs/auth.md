# Sign-in, hidden pages, and admin controls

Google OIDC login gates hidden pages (currently `/motorcycles` and `/podrick`) and
admin-only controls on public pages. Identity is a 30-day encrypted
`__Host-viewer` cookie (`src/app/login.rs`); authorization is
`src/content/access.rs`, checked on every request.

## Allowlisting someone

Sign in as the admin and use `/admin/permissions`: one form set per hidden
page — add an email to grant, a revoke button beside each grant. Grants are
rows in the `hidden_page_grants` table, checked on the request that needs
them, so granting and revoking apply on the target's very next request — no
redeploy, no session state to invalidate.

Allowlists are database-only, NEVER committed: the repo is public, so a
friend's grant in `src/content/access.rs` would publish their email to git
history forever. `ADMIN_EMAIL` is the deliberately committed, app-wide
administrator: it sees every hidden page without being granted, and it is
the only identity `/admin/permissions` opens for. The login callback
refuses to mint a cookie for emails holding no grants, so strangers who
find `/login` end up holding nothing.

When the database is unreachable, grant checks fail closed: hidden pages
404 for signed-in non-admins, the login callback reports "no access", and
the admin page says the store is unreachable instead of rendering every
list empty. The admin rule is a constant comparison and survives outages.

A grant opens only the named hidden page. It never grants admin
capabilities. For example, `/lifting` renders its "upload lift" dialog only
when the signed-in email matches `ADMIN_EMAIL`, and `POST /lifting/upload`
independently repeats that exact check before reading the request body.

## Admin pages (`/admin`, `/admin/permissions`)

`/admin` is the tool index — a rail of cards fed by
`ADMIN_TOOLS` in `src/app/admin.rs`; the "admin" link the shell adds to the
admin's own footer line is its one listing. Admin pages are tooling, not
hidden pages: they stay out of every registry including `HIDDEN_PAGES`, and
follow the hidden-page invariants below (`no-store` before `shell()`,
`analytics: false`, signed-out → login redirect, signed-in non-admin → the
real 404). `/admin/permissions` manages the grants; its grant/revoke POSTs repeat the
admin check, require positive same-origin evidence, and bound the body
before parsing. Grants must target a registered `HIDDEN_PAGES` path;
revokes also accept de-registered paths, which the page lists in a
"no longer registered" section so removing a page never strands its grant
rows invisibly. Emails are stored trimmed and lowercased
(`access::normalize_email`); the schema ASSERTs in `src/schema.surql`
mirror that validation as a backstop.

`/diary` (`src/app/diary.rs`) is admin-only in the same way but lives at
its own path: the admin's private diary, database-backed
(`diary_entries`), deliberately NOT a `HIDDEN_PAGES` entry — a registry
entry would render a grant form for it here, and one mistyped grant would
share a diary. Its `/admin` tool card is its one listing; entry permalinks
reuse the lifting archive's Eastern public-path shape as record keys.

## The diary as an installable offline app (PWA)

`/diary` installs on Android as a standalone app with an offline write
queue. `src/app/pwa.rs` serves the stable pieces (`/sw.js`,
`/diary.webmanifest`, two icons); the diary pages load
`src/app/diary/diary.js`; the worker `src/app/diary/sw.js` registers with
scope `/diary`, so no public page is ever controlled and the CDN
invariants above are untouched. The queue logic itself is Rust —
`crates/diary-core` compiled to wasm, storing entries in a device-local
SurrealDB over IndexedDB, served by `src/app/diary_sync.rs` — read
`docs/diary-sync.md` alongside this section before touching any of it.
Invariants:

- The PWA routes are deliberately ungated: Chrome fetches manifests
  without credentials (a cookie gate breaks install), and the bytes only
  disclose that a diary app exists — which the public repo and the login
  redirect already do. Keep anything private out of them.
- With JS, saves are local-first (the wasm store) and synced ONLY by the
  service worker: `POST /api/diary/entries` + `GET /api/diary/snapshot`
  (admin-gated, `no-store`), or straight to the database over a websocket
  when direct sync is flagged on (docs/diary-sync.md). Never add a
  page-side POST: one sync implementation — `diary_core::sync::run` over
  the outbox — is what keeps replays and pulls safe (a garbage pull
  response must be a mirror no-op, never "zero entries"). Without JS (or
  when the wasm store refuses: no `wasm-dist/` build served, private-mode
  IndexedDB), the plain form POST to `/diary/write` still works. The
  pre-wasm IndexedDB queue (`diary-queue`) is drained by a one-way
  migration in the worker.
- The API starts placement from the CLIENT's composition second — not the
  time the entry syncs — inside a bounded window (a year back, 5 minutes
  forward; outside is a 422, never clamped, because clamping would mint a
  fresh key per replay and double-post). At every candidate second,
  identical Entry Content is "already saved" and different content probes
  forward ≤5 s. A device collision may re-anchor the queued entry before
  sync, and a server collision may re-anchor it again. Overwriting an entry
  stays impossible.
- Replies carry an optional Reply Target in Entry Content. It is a soft,
  valid-permalink Entry Reference, not a SurrealDB record link: acceptance
  validates the key shape but does not require a target row to exist. For now,
  only a synced Device Entry offers the reply action. A pending Device Entry's
  predicted Entry Key is not selectable until delivery or a snapshot confirms
  it as an Entry Reference.
- Offline reads are deliberate, device-local, and Ben's choice — and they
  cover the WHOLE diary: the local store mirrors every entry, and when a
  navigation's network fetch fails the worker RENDERS the page from that
  mirror (the same Rust router and views the server runs, compiled to
  wasm) — pagination, permalinks, and pending rows included. There is no
  cached-HTML copy anymore; only hashed assets and the versioned wasm
  pair sit in Cache Storage. The mirror, those caches, and the queue
  OUTLIVE sign-out and `COOKIE_KEY` rotation — a signed-out or stolen
  device reads the full diary offline; wiping it means clearing the
  site's data in Chrome on the device. That is the accepted trust model:
  device possession is the boundary, and the server stays the auth
  boundary whenever it is reachable (offline SSR answers only after the
  network fetch fails).
- A flush stops and keeps the queue on 401/404 (sign in again) and on
  400/403/5xx/network trouble (retry later); a 400 can be an older server
  rejecting a newer strict envelope during rollout. Every HTTP sync request
  and direct JWT carries the exact Diary Schema Epoch; a mismatch is 503 and
  leaves the outbox untouched. Ordered migrations put both stores in one
  current shape. The page and worker serialize local epoch checks and store
  operations with the `diary-store` Web Lock, while an older server binary
  refuses a newer diary migration ledger instead of reapplying obsolete
  permissions. Only
  409/413/415/422 mark an entry failed, and failed text stays on the page
  with a discard button — queued diary text is never silently dropped.
- `/sw.js` and the manifest keep stable un-hashed URLs (a worker's URL is
  its identity; a hashed URL would register a new worker every deploy).
  Served `no-cache` / day-long cache respectively via `pwa.rs`.

## Adding a hidden page

Copy the `src/app/motorcycles.rs` pattern (+ `mod` in `app.rs`), add a
display entry to `HIDDEN_PAGES` in `src/content/access.rs`, grant access
at `/admin/permissions` (the new entry's form set appears there
automatically). Hidden pages deliberately stay OUT of
`INTERESTS`/`POSTS`/`site_routes()` — that's what keeps the nav, indexes,
feed, 404, and analytics trackability silent about them. The
`HIDDEN_PAGES` entry is the page's only listing: the shell's tmux
windows and the `~` listing at `/` render it for allowlisted viewers,
nobody else.
Invariants:

- `no-store` before `shell()` on every variant, including the not-found
  branch — the shell's default `s-maxage=86400` would let Cloudflare cache
  one viewer's HTML (or the 404) for everyone for a day. The response
  layer (below) already covers requests bearing the viewer cookie; the
  page-level header is deliberate redundancy for a page whose every
  variant must stay uncached.
- `analytics: false` on `shell()` — the analytics dashboard is public.
- Path must not be analytics-trackable: nothing under `/felix/`,
  `/swing/`, or `/lifting/` (`is_trackable_route`), or the hidden path
  shows up as a referrer of public pageviews. The `HIDDEN_PAGES` test
  enforces this.
- Signed-out → `Err(redirect("/login?next=…"))`. This reveals that *a*
  door exists at the path, in exchange for friends being able to just
  follow a link; render `not_found_page` for the signed-out case too if a
  page's existence is itself the secret.
- Signed-in but not allowlisted → `not_found_page`, indistinguishable from
  absence in the document — though not in headers: the denial 404 is
  `no-store` while real 404s ride the cacheable shell default. Cache-safety
  wins that tradeoff; don't "fix" it by making the denial cacheable.
- Browser POST routes repeat their authorization checks; a hidden form or
  control is not an authorization boundary. They also require positive
  same-origin evidence and bound the body before parsing it.

## Signed-in rendering and the CDN

The shell personalizes for viewers: allowlisted hidden pages join the
tmux windows and the `~` listing, and the quiet "log in with google" link
at the footer's bottom right becomes a "signed in as … · sign out" line — for
the admin it also carries the `/admin` link. Personalized
HTML must never be edge-cached, so the site-wide response layer
(`src/app/response_layer.rs`) forces `Cache-Control: private, no-store`
on any request carrying a `__Host-viewer` cookie — keyed on presence,
not validity, so a garbage cookie fails closed; framework error
responses (404/405/redirects/500s) are converted inside the layer so
they can't escape the stamp. The one exemption is by response, never
request path: hashed assets declare `immutable` and stay cacheable
(`/_topcoat/junk` falls through to the catch-all 404, which renders the
personalized shell — a path-based exemption would leak it). Pages keep
declaring the cache headers their anonymous renders want; the layer
overrides only for cookie-bearing requests. Topcoat allows ONE discovered `#[layer]` per
path (a second `#[layer("/")]` panics at router build, which `just
check` does not catch) — new site-wide response behavior goes in that
file, never a sibling layer.

Cloudflare serves cached HTML without consulting request cookies, so a
signed-in visitor would get the anonymous copy until it expires — never
a leak (personalized copies are `no-store` and thus never stored), just
missing personalization. Fix at the zone: a Cache Rule with custom
filter `http.cookie contains "__Host-viewer"` → Bypass cache, placed
after the eligible-for-cache rule (later cache rules win).

## OAuth mechanics (`src/app/login.rs`)

- Authorization-code flow + PKCE + `state`, both parked in an encrypted
  10-minute `__Host-google-flight` cookie during the Google round-trip.
- The `id_token` signature is deliberately unchecked: the token arrives
  directly from Google's token endpoint over TLS, which OIDC permits for
  confidential clients. Issuer, audience, expiry, and `email_verified` are
  still validated, and emails are lowercased before allowlist checks.
- Routes that touch the cookie jar return hand-built `Ok(303)` responses —
  the topcoat cookie layer only flushes `Set-Cookie` on `Ok`, so
  `Err(redirect(…))` would silently drop the write.

## Environment

- `COOKIE_KEY` — any secret string ≥32 bytes (`openssl rand -hex 32`).
  Unset: the key is ephemeral and viewer sessions reset every restart
  (fine until sign-in matters). Rotating it signs everyone out — that is
  the "log everyone out now" lever.
- `GOOGLE_OAUTH_CLIENT_ID` / `GOOGLE_OAUTH_CLIENT_SECRET` — unset:
  `/login` reports sign-in unconfigured; hidden pages still 404/redirect.
- Hidden-page allowlists are `hidden_page_grants` rows behind the
  `SURREALDB_*` variables (railway-deploy.md), not an env var. Database
  unreachable: hidden pages are admin-only.
- `SITE_ORIGIN` — already set in prod; the callback redirect URI is
  `$SITE_ORIGIN/auth/google/callback`.

## Creating the Google OAuth client (one-time, dashboard)

1. console.cloud.google.com → a project → APIs & Services → OAuth consent
   screen: External, then publish. The `openid`/`email` scopes are
   non-sensitive, so publishing needs no Google verification review.
2. Credentials → Create credentials → OAuth client ID → Web application,
   with authorized redirect URIs
   `https://benjisponge.com/auth/google/callback` and
   `http://localhost:3000/auth/google/callback` (dev).
3. Set the id/secret on the Railway web service; for local testing copy
   `.env.dev.example` → `.env.dev`, fill both in, then `just dev`
   (`scripts/dev.sh` sources `.env.dev` and pins a fixed dev
   `COOKIE_KEY` / local SurrealDB).

Dev sign-in works in Chrome/Firefox only: the auth cookies are
`__Host-`/`Secure`, which those browsers accept from `http://localhost`
but Safari silently drops — every attempt loops to "sign-in expired".

## First deploy of a new hidden page

The edge may hold a day-old cached 404 for the page's URL from before it
existed — run `just deploy` (zone purge) after shipping it.
