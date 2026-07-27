# Sign-in, hidden pages, and admin controls

Google OIDC login gates hidden pages (currently `/motorcycles`) and
admin-only controls on public pages. Identity is a 30-day encrypted
`__Host-viewer` cookie (`src/app/login.rs`); authorization is
`src/content/access.rs`, checked on every request.

## Allowlisting someone

Edit the `HIDDEN_PAGE_ACCESS` variable on the Railway web service (which
redeploys) — entries `;`-separated, emails `,`-separated:

```text
HIDDEN_PAGE_ACCESS=/motorcycles:alice@gmail.com,bob@gmail.com;/garage:carol@example.com
```

Allowlists are env-only, NEVER committed: the repo is public, so a
friend's grant in `src/content/access.rs` would publish their email to git
history forever. `ADMIN_EMAIL` is the deliberately committed, app-wide
administrator and sees every hidden page without being listed. The login
callback refuses to mint a cookie for emails appearing nowhere, so
strangers who find `/login` end up holding nothing.

`HIDDEN_PAGE_ACCESS` grants access only to the named hidden page. It never
grants admin capabilities. For example, `/lifting` renders its "upload lift"
dialog only when the signed-in email matches `ADMIN_EMAIL`, and
`POST /lifting/upload` independently repeats that exact check before reading
the request body.

## Adding a hidden page

Copy the `src/app/motorcycles.rs` pattern (+ `mod` in `app.rs`), add a
display entry to `HIDDEN_PAGES` in `src/content/access.rs`, grant access
via `HIDDEN_PAGE_ACCESS`. Hidden pages deliberately stay OUT of
`INTERESTS`/`POSTS`/`site_routes()` — that's what keeps the nav, indexes,
feed, 404, and analytics trackability silent about them. The
`HIDDEN_PAGES` entry is the page's only listing: the shell's interests
dropdown and `/interests` render it for allowlisted viewers, nobody else.
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
interests dropdown and `/interests`, and a quiet "signed in as … · sign
out" line sits at the footer's bottom right of every page. Personalized
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
- `HIDDEN_PAGE_ACCESS` — the hidden-page allowlists (format above). Unset:
  hidden pages are admin-only. It does not grant admin-only controls.
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
