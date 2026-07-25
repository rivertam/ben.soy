# benjisponge.com

Rust SSR personal site on topcoat 0.3.0 — a niche framework; read
`docs/topcoat-notes.md` before writing any topcoat code, don't guess APIs.

## Commands

- `just dev [port]` — start local Postgres (docker) + live-reload server (default 3000); run `just reset-fitness-local [csv]` separately to rebuild fitness data; details in `docs/fitness.md`
- `just build` — cargo build + `topcoat asset bundle`; serving without the bundle step panics
- `just check` — fmt + clippy -D warnings + tests; must pass before claiming done
- `just deploy` — optional Railway redeploy + Cloudflare cache purge (Railway GitHub App deploys on push to main; CI only runs `just check`). Prod path: `docs/railway-deploy.md` (Dockerfile only — not Railpack); DNS/Tunnel/CDN: `docs/cloudflare-deploy.md`
- `just sync-spire [--dry-run|--json]` — upload new Slay the Spire 2 runs from this machine's save files to the site's database; idempotent; pipeline details in `docs/railway-deploy.md`
- `just sync-fitness <csv> [--dry-run|--json]` — idempotent fitness CSV upload; read `docs/fitness.md` before changing its data flow or taxonomy

## Adding a page

- Post: `src/app/thoughts/<slug>.rs` + `mod <slug>;` in `thoughts.rs` + entry in `src/content/posts.rs`
- Interest: `src/app/interests/<name>.rs` (copy one; it pulls its copy via `interest("<name>")`) + `mod <name>;` in `interests.rs` + entry in `src/content/interests.rs`
- Other fixed page: also add its route to `src/content/routes.rs::site_routes()`
- Nav, indexes, and 404 all derive from these registries — touch nothing else

## Gotchas

- A `#[page]` module not declared in its parent `mod` silently doesn't route
- Tailwind classes are scanned from `.rs` files at build time; a class rendering unstyled means a stale scan: `touch styles/input.css && cargo build`
- Prose lives in Rust string literals; a `\` continuation eats the newline and the next line's leading spaces, so keep the word-space before the `\`; escape `"`
- `styles/planes-charts.css` hardcodes generated `seg-<bar>-<slice>` and `data-pick-*` names; the css tests in `charts.rs` are the tripwire
- After editing `reference_data.rs` run `just test` — unknown source/option/activity ids panic at render time, tests catch them first
- `?oneway` is presence-only and `trip=oneway` also parses (share-URL back-compat) — don't simplify
- `emissions.rs` deliberately models only the myclimate fuel curve; the missing aircraft-production and infrastructure terms are not an omission to complete
- Units: kg CO₂e and km everywhere; number formatting mirrors Intl.NumberFormat half-away-from-zero — don't "fix" the rounding
- Spire runs are data, not content: `/`, `/spire`, `/feed.xml` render them live from `/api/spire/runs` — publish runs with `just sync-spire`, never by editing the repo
- Fitness sets are database data, not content: never hardcode the CSV; changes spanning `/lifting`, import, API, schema, tags, or local startup must preserve `docs/fitness.md` invariants
- Records (`/lifting` badges) are derived from set history at snapshot build (`src/app/interests/lifting/archive/records.rs`), never stored or imported — there is deliberately no records table
- Muscle-map primary/secondary (`lifting/muscles.rs`) is likewise derived at render — no rank column; a taxonomy change in `exercise_tags()` must keep `PRIMARY_BY_MOVEMENT` aligned
