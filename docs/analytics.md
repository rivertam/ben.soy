# Analytics

`/analytics` is a public, server-rendered instrument panel backed entirely by
SurrealDB through its Rust SDK. Collection and identity writes terminate in
the Topcoat application; there is no analytics Worker or third-party
collector.

## Request flow

Cached HTML pages load a small, modern-browser `analytics.js` sensor. It reports
to `POST /api/analytics/events`, an uncached route already forwarded to Topcoat
by the generic `/api/*` path. All interpretation, sessionization, validation,
storage, aggregation, and rendering remain in Rust.

The sensor emits three event kinds:

- `pageview` — a privacy-reduced acquisition referrer, timezone, and viewport;
  Rust derives the canonical route, campaign labels, language, local clock, and
  coarse request dimensions
- `engagement` — cumulative visible time, maximum reading depth, LCP, Core Web
  Vitals session-window CLS, and navigation duration
- `outbound` — destination hostname

The database atomically rotates the session after 30 minutes without received
activity. Topcoat issues a 400-day `Secure`, `HttpOnly`, `SameSite=Lax`,
`__Host-` visitor cookie. Only its SHA-256 token hash is stored. A per-tab
random bootstrap nonce and small alias table make simultaneous first-load
beacons converge on the same anonymous visitor even before the cookie response
arrives, including across a rapid navigation. The sensor removes its transient
storage after a successful response; the nonce is not stored on event rows and
never defines a session or a fallback identity when cookies are unavailable.

One document reuses its engagement event id. Visibility changes and BFCache
cycles can safely flush a newer cumulative snapshot; each write only raises
the stored measurements, so reading time and sample counts are not duplicated.

## Data boundaries

The application never stores:

- an IP address
- a raw user agent
- a query string or fragment, except the bounded `utm_source`, `utm_medium`,
  and `utm_campaign` allowlist
- an external referrer path
- an outbound destination path
- a precise coordinate

Country comes from a validated `CF-IPCountry` header when Cloudflare supplies
one. Maps use a self-hosted, approximate country-centroid SVG; no visitor
request goes to a map vendor.

External referrers are reduced to hostnames in the browser before transport.
Internal referrers retain only a known site path. The 404 page does not load the
tracker, and the API accepts only canonical fixed routes or the bounded
one-segment gallery/workout route shapes.

Global Privacy Control, Do Not Track, and WebDriver opt out before browser
observers or listeners initialize. Topcoat independently enforces the GPC and
DNT request headers before parsing a passive event, creating a cookie, or
opening the database. The private ledger remains an explicit user-directed
form submission rather than passive tracking.

## Public and private surfaces

The public dashboard queries only anonymous events. Geography, referrer hosts,
technology, journeys, outbound hosts, campaigns, and dynamic page paths are
suppressed until at least three anonymous visitors contribute. This is a
small-cohort display rule, not proof that public traffic is human or resistant
to a determined Sybil attack.

The “private ledger” form writes to a separate identity table keyed through the
hardened visitor cookie. Its hidden, ephemeral bootstrap nonce closes the same
first-response race without accepting a visitor hash or analytics session id.
It has no public read route, no dashboard join, and submitted values are never
echoed or included in application logs. A name is an unverified, voluntary
label—not authentication.

## Database behavior

Bounded table and field definitions live in `src/schema.surql`; query-specific
indexes live in the forward-only migrations under
`src/data/schema_migrations/`. Rust row shapes live in
`src/app/analytics/models.rs`.

Dashboard windows are 7, 30, 90, or 365 UTC days. Each render performs one
database statement with a three-second application-side timeout. It returns
the bounded event rows plus scalar markers for matching sessions whose first
pageview predates the window, then aggregates every panel in Rust. The markers
keep acquisition cohorts from counting a session again when it straddles a
window boundary; the prior-session probe only looks back one idle window
(30 minutes), which is enough because a surviving session id must have had
activity inside that gap or it would already have rotated. An unhealthy
database falls back to the standby card. Only canonical range URLs execute
the query, preventing arbitrary query strings from bypassing shared caching.

Raw anonymous events, current session cursors, cookie aliases, and voluntary
identities are retained until deliberately removed. Public reads are bounded to
365 days; no raw data is rewritten into lossy rollups.

## Write hardening

Event and identity handlers require positive same-origin browser evidence and a
matching current-page Referer. Rust derives the route and allowlisted UTM
labels from that current-page header; the separate body referrer is the
privacy-reduced acquisition source. Handlers enforce exact content types,
streamed body limits, strict event-specific field sets, UUID-v4 idempotency,
bounded dimensions, canonical paths, and database constraints. Responses are
`no-store`, do not enable CORS, and never disclose whether a private identity
already exists.

The in-process request guard limits accidental loops and low-effort floods. It
is defense in depth rather than a distributed rate limiter; idempotency,
constraints, cohort suppression, bounded queries, and short database timeouts
remain authoritative when the app has multiple containers.

Alias resolution, session rotation, and identity writes each use one atomic
upsert. Engagement updates use an explicit transaction so the cumulative event
maximum and its matching session activity cursor advance together. Preserve
those atomicity boundaries when changing analytics storage.

## Local verification

`just dev` starts local SurrealDB before Topcoat; the app applies
`src/schema.surql` plus pending site/diary migrations on its first data-backed
connection. After schema or analytics changes:

```sh
just build
just check
```

The asset-bundle step in `just build` is required before serving the binary.
