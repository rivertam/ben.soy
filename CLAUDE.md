# benjisponge.com

Rust SSR personal site on topcoat 0.5.0 — a niche framework; read
`docs/topcoat-notes.md` before writing any topcoat code, don't guess APIs.
Same rule for the data layer: read `docs/surrealdb-notes.md` before touching
SurrealDB models, queries, or schema (Rust SDK and server pinned to 3.2.3).

## Commands

- `just dev [port] [--no-podrick] [--podrick-reset]` — start local SurrealDB (docker) + live-reload server (default 3000), plus Podrick when `.env.dev` configures it (`docs/podrick.md`); run `just reset-fitness-local [csv]` separately to rebuild fitness data; details in `docs/fitness.md`
- `just build` — cargo build + `topcoat asset bundle`; serving without the bundle step panics
- `just wasm` — build the diary queue's wasm module into `wasm-dist/` (optional: without it /diary falls back to plain form POSTs); the wasm-bindgen CLI version must equal diary-worker's pinned crate version; read `docs/diary-sync.md` before touching `crates/` or the sync routes. `crates/diary-worker` is deliberately its OWN workspace carrying a `[patch.crates-io]` on surrealdb-core (wasm-only upstream bug) — never merge it into the root workspace or the server would build patched code
- `just check` — fmt + clippy -D warnings + tests; must pass before claiming done
- `just deploy` — optional Railway redeploy + Cloudflare cache purge (Railway GitHub App deploys on push to main; CI only runs `just check`). Prod path: `docs/railway-deploy.md` (Dockerfile only — not Railpack); DNS/Tunnel/CDN: `docs/cloudflare-deploy.md`
- `just sync-spire [--dry-run|--json]` — upload new Slay the Spire 2 runs from this machine's save files to the site's database; idempotent; pipeline details in `docs/railway-deploy.md`
- `just sync-fitness <csv> [--dry-run|--json]` — idempotent fitness CSV upload; read `docs/fitness.md` before changing its data flow or taxonomy
- `just delete-lift <path|url> [--api <origin>] [--yes]` — the archive's only destructive operation: `DELETE /api/fitness/workouts/by-path/{path}`; contract in `docs/fitness.md`
- `just podrick once|run [--dry-run]` — the Discord bot, defaulting to the PRODUCTION api; `just podrick-local` is the local-stack twin (sources `.env.dev`, pins local db + api) and `just dev` already runs it. Read `docs/podrick.md` before touching `src/app/interests/podrick/`

## Adding a page

- Post: `src/app/thoughts/<slug>.rs` with `register_post!(...)` metadata + `mod <slug>;` in `thoughts.rs`; `/thoughts`, `/log`, `/feed.xml`, routes, and footer count derive from the distributed post registry
- Interest: `src/app/interests/<name>.rs` (copy one; it pulls its copy via `interest("<name>")`) + `mod <name>;` in `interests.rs` + entry in `src/content/interests.rs`
- Other fixed page: also add its route to `src/content/routes.rs::site_routes()`
- Nav, indexes, and 404 all derive from these registries — touch nothing else
- Login-gated hidden page: copy `src/app/motorcycles.rs` + display entry in `access.rs::HIDDEN_PAGES` (its only listing — the shell's tmux windows, the `~` listing at `/`, and `/admin/permissions` all derive from it); allowlist = `hidden_page_grants` DB rows managed at `/admin/permissions`, never committed (public repo); keep it OUT of the registries above; read `docs/auth.md` first

## Gotchas

- A `#[page]` module not declared in its parent `mod` silently doesn't route
- Tailwind classes are scanned from `.rs` files at build time; a class rendering unstyled means a stale scan: `touch styles/input.css && cargo build`
- Prose lives in Rust string literals; a `\` continuation eats the newline and the next line's leading spaces, so keep the word-space before the `\`; escape `"`
- `styles/planes-charts.css` hardcodes generated `seg-<bar>-<slice>` and `data-pick-*` names; the css tests in `charts.rs` are the tripwire
- After editing `reference_data.rs` run `just test` — unknown source/option/activity ids panic at render time, tests catch them first
- `?oneway` is presence-only and `trip=oneway` also parses (share-URL back-compat) — don't simplify
- Layover `via` params are hand-parsed from the raw query (`parse_vias`), never declared in `PlanesQuery` — serde errors on repeated declared keys and the error redirect would wipe the query
- `emissions.rs` deliberately models only the myclimate fuel curve; the missing aircraft-production and infrastructure terms are not an omission to complete
- Units: kg CO₂e and km everywhere; number formatting mirrors Intl.NumberFormat half-away-from-zero — don't "fix" the rounding
- Spire runs are data, not content: `/log`, `/spire`, `/feed.xml` render them live from `/api/spire/runs` — publish runs with `just sync-spire`, never by editing the repo
- Fitness sets are database data, not content: never hardcode the CSV; changes spanning `/lifting`, import, API, schema, tags, or local startup must preserve `docs/fitness.md` invariants
- Records (`/lifting` badges) are derived from set history at snapshot build (`src/app/interests/lifting/archive/records.rs`), never stored or imported — there is deliberately no records table
- Fitness writes are create-only except `DELETE /api/fitness/workouts/by-path/{path}` (sync token OR admin cookie + same-origin) and the admin weight editor `POST /lifting/exercise/{name}`. Deletes remove the workout and its sets only — `exercises`/`exercise_tags`/`exercise_muscles` orphans are left deliberately (invisible to every count, and they preserve corrected taxonomy/weights across a delete-and-repaste)
- Muscle credit comes from `exercise_muscles` ratio rows (granular 28-muscle vocabulary in `lifting/muscle_taxonomy.rs`; schema ASSERT lists are test-enforced against it). Seeding is insert-only per exercise (`reconcile_muscle_weights`) — any existing row, especially `source='admin'`, blocks reseeding; only the exercise-page form replaces rows. Map shading/load panels derive at render (threshold 75 = primary) — no rank column. Deletes on `exercise_muscles` use `=` per pair, never `IN [..]`
- The cookie layer drops `Set-Cookie` on `Err` responses — auth routes build `Ok(303)`s by hand; gated pages emit `no-store` before `shell()` or the edge caches one viewer's HTML for a day (`docs/auth.md`)
- Topcoat discovery allows ONE `#[layer]` per path — a second `#[layer("/")]` panics at router build, and `just check` doesn't boot the router; whole-site response behavior (viewer no-store, em-dash links) all lives in `src/app/response_layer.rs`
- Signed-in HTML is personalized (shell nav/footer), so `response_layer.rs` forces `private, no-store` when the `__Host-viewer` cookie is present, and prod needs the Cloudflare bypass-on-cookie Cache Rule (`docs/auth.md`) — without it signed-in visitors see the cached anonymous page
- `/diary` is a local-first offline PWA: ONE local table is outbox and mirror (a queued entry is a `state='pending'` row, flipped in place on delivery — never deleted mid-flush), entry ids/permalinks are predicted at enqueue with the same probe the server runs, and the service worker owns ALL syncing (flush-then-pull in one Web Lock hold; direct-to-SurrealDB when the `DIARY_SYNC_*` flag trio is set) AND renders /diary offline via `Router::handle` in wasm. Transcript markup has exactly one definition — `diary_core::views` — used by the server page, the worker SSR, and the page JS via the served `<template id="diary-bubble">`; never hand-build bubble DOM. Read `docs/diary-sync.md` + the PWA section of `docs/auth.md` before touching any of it
- Hand-served byte routes (`favicon.rs`, `pwa.rs`) must set Content-Type explicitly — the response layer treats untyped bodies as HTML and runs the em-dash rewriter through them; `/sw.js` must stay a stable un-hashed URL because a service worker's URL is its identity
- Tailwind scans only `.rs` files (including `crates/`); JS must never carry class strings — the diary page clones its served `<template>` instead. One `asset!` (or `tailwind::stylesheet!()`) invocation per file TOTAL: a second registers a duplicate route and panics at router BUILD, which `just check` never runs — share `chrome::SITE_CSS` / `diary::DIARY_JS`
- Podrick ships a fourth Railway service plus the hidden `/podrick` page; read `docs/podrick.md` before changing it. Never move or clear production `podrick_meta` cursors: they suppress lift-history announcements and Pants-history reactions. Keep `--dry-run` write-free; only `just dev --podrick-reset` may clear LOCAL Podrick state, and the binary must never grow a reset/backfill switch
- SurrealDB `CREATE` returns `id` as a record id and `SELECT *` drops `option` fields holding `NONE`; both silently break deserialization into `String`/`Option` model fields — see the result-shape rules in `docs/surrealdb-notes.md`
