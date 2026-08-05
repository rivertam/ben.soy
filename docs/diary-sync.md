# The local-first diary (client-side SurrealDB + topcoat SSR over wasm)

The /diary system is written in Rust on both sides of the wire, and the
larger idea it started as — Remix-style loader/clientLoader isomorphism for
a topcoat + SurrealDB app — is BUILT: one local table is both outbox and
mirror; entry ids (permalinks) are predicted at enqueue with the same probe
the server runs; the transcript markup is ONE set of pure components
rendered by the server page, by the service worker's offline SSR
(`Router::handle` inside the worker, the topcoat 0.5.0 serve split), and
cloned by the page JS from a served `<template>`; sync is flush-then-pull
through a two-method transport trait whose direct implementation is just
another `Surreal<Any>` handle. The page JS cannot tell which renderer drew
the HTML it reconciles against — that sentence is the whole design.

## Shape

- `crates/diary-core` — everything shared. `contract` is the wire protocol
  (`WireEntry`, the snapshot shapes, response/pull classification);
  `eastern` the America/New_York projection (moved from the lifting
  archive; permalink keys derive from it); `store` the entry model, key
  projection, and probe-and-dedupe queries; `outbox` the DEVICE-LOCAL
  single store — mirror and queue in one `diary_entries` table, a queued
  entry just a row with `state = 'pending'` that flips in place on
  delivery; `sync` the flush-then-pull pass over the two-method `Remote`
  trait (`HttpRemote` lives in the worker; `DirectRemote` is a raw
  `Surreal<Any>`), with the empty-snapshot wipe guard; `views` (feature
  "view") the PURE components — transcript, bubble, compose, template,
  minimal offline chrome — zero awaits, which is what satisfies topcoat's
  `+ Send` render bounds on wasm for free. All of it runs against the SAME
  `Surreal<Any>` handle `src/data.rs` uses. `cargo test -p diary-core`
  exercises everything natively against `mem://` (including two-device
  convergence walks and store-to-store direct sync); the phone runs the
  identical code against `indxdb://diary`.
- `crates/diary-worker` — the wasm binary: the store exports
  (`diary_enqueue/snapshot/discard/import`), `diary_sync` (one
  flush-then-pull pass; picks direct or HTTP transport), and
  `diary_render` — a serve-less topcoat router (`features = ["router",
  "view", "discover"]`, no hyper anywhere) whose `#[page]` fns render the
  mirror through the shared views; store reads bounce through
  `spawn_local` + a oneshot channel because render futures must be `Send`
  and indxdb futures are not. Deliberately its own cargo workspace,
  EXCLUDED from the root — see the patch section below. `just check` and
  CI never touch it: breakage surfaces at `just wasm` or the Docker
  wasm-builder stage. (The crate root is `cfg(target_arch = "wasm32")`-
  gated so a stray native build compiles an empty crate.)
- `src/app/diary.rs` — auth and routing glue over the SAME
  `diary_core::{store,views}` calls the worker makes, plus the flag-gated
  token mint. One definition of protocol, queries, and markup; a drift is
  a type error, not a silent half-parse.
- `src/app/diary_sync.rs` — serves `wasm-dist/` at three stable routes
  (see Serving below); its loader also carries the hashed asset URLs the
  offline SSR links and the direct-sync endpoint when flagged.
- `src/app/diary/sw.js`, `diary.js` — browser glue only: worker lifecycle,
  asset cache, Web Lock, Background Sync, BroadcastChannel, and the page's
  template-clone bubble handling keyed by `data-id`.

## Stable ids and flush semantics

An entry's id — its permalink — is predicted at ENQUEUE with the identical
probe-and-dedupe loop the server runs (`store::save_entry` vs the outbox's
local placement): same second + same body at any probed key is the same
entry (a double-tap converges instead of minting a twin the server would
store twice); a different body probes the second forward. The permalink is
right from birth, the server's own probe stays as the cross-device
backstop, and page reconciliation collapses to "does the DOM already show
this data-id" against the server-shipped `<template id="diary-bubble">` —
the old five-bucket painter and provisional map are gone.

Flushes send oldest-first by enqueue time; stop on auth (401/404) or
retryable trouble (network, 403, 5xx, captive-portal 200) so composition
order survives; permanent rejections (400/409/413/415/422) mark the entry
failed IN PLACE and keep its text for manual copy. A delivered entry flips
`pending -> synced` in place too — never deleted — so no snapshot can watch
a message blink out of existence mid-flush. The `FlushReport` still carries
`saved_entries` (local id -> server identity); it matters only for the rare
server bump, where the local row is re-keyed to the server's id (and if
that key is held by a DIFFERENT pending row, the delivered row is simply
released — its text is safe server-side and the pull that follows in the
same lock places it). The dashed queued styling and "will sync" label
appear only when a report says the queue is actually blocked; "failed" only
when the server rejected the entry.

`outbox::flush` takes the transport as a generic closure with deliberately
NO `Send` bounds: browser futures are `!Send`, native test futures don't
care, and wasm is single-threaded anyway. Adding `Send` there would break
the wasm build; this is the one signature where the isomorphism is fragile.

## Legacy migration

The pre-wasm queue (IndexedDB `diary-queue`/`entries`) is drained by the
worker on each flush, under the flush Web Lock: read all → `diary_import`
(idempotent by `(written_at, body)`, state/reason preserved, bodies kept
byte-for-byte) → delete legacy rows only after import returns. A crash
anywhere re-runs safely. The emptied database is left behind for any
straggler old worker. Page kicks are deliberately unconditional (the old
"any pending?" check is gone) because only the worker can see both stores
while the migration exists.

The v1 wasm queue (`diary_outbox`, the separate outbox table before the
single store) drains the same way, inside `outbox::open` itself: rows port
into `diary_entries` under predicted keys (unprojectable timestamps land
under synthetic `failed-*` keys as failed rows — never dropped), then the
old rows delete one by one. It is a STANDING step, not one-shot: during
deploy skew an old worker happily re-creates the table.

## Serving and cache pairing

`just wasm` (or the Dockerfile's wasm-builder stage) puts two artifacts in
`wasm-dist/`: `diary_sync.js` (wasm-bindgen `--target no-modules` glue) and
`diary_sync_bg.wasm`. They only work as a matched pair, so `diary_sync.rs`
never serves them unversioned to callers:

- `/diary-sync.js` — two-line loader, `no-cache`. Sets `self.DIARY_SYNC`
  with `?v=<hash>` URLs for the pair; the hash covers BOTH files.
- `/diary-sync-glue.js`, `/diary-sync_bg.wasm` — `immutable` only under the
  exact current `?v=`; any other query answers `no-cache` so a deploy race
  can never pin wrong bytes under a year-long key.

The service worker `importScripts` the loader and then the glue at
evaluation time (Chrome refuses new importScripts URLs after install), so a
loader byte change on deploy is also what triggers the worker update. There
is deliberately no try/catch around those imports: if the pair can't load,
this worker version fails to install and the previous working version keeps
running. The page loads the same two files via injected classic scripts and
falls back to the plain form POST when anything refuses — a dev checkout
without `wasm-dist/` behaves exactly like the no-JS diary.

Artifacts are read from disk per request with a file-stamp cache, not
`include_bytes!`: `just build` stays green without a wasm toolchain and a
running `just dev` picks up a fresh `just wasm` immediately. The immutable
variants are exactly what `response_layer.rs`'s signed-in exemption expects;
everything else stays `no-store` for cookie-bearing requests.

## Building

- `just wasm` — needs `rustup target add wasm32-unknown-unknown` and
  `cargo install wasm-bindgen-cli --version 0.2.126 --locked`. The CLI
  version MUST equal diary-worker's pinned `wasm-bindgen` crate version;
  mismatches fail loudly at bindgen time.
- `.cargo/config.toml` passes `--cfg getrandom_backend="wasm_js"` for the
  wasm target only (getrandom refuses to guess a randomness source there).
- `[profile.wasm]` (in the worker's own manifest) is size-first
  (`opt-level = "z"`, fat LTO, `panic = "abort"`). Current output ≈19.1 MB
  raw / ≈5.1 MB gzipped — the embedded SurrealDB engine is the floor;
  topcoat's router+views added ≈0.6 MB and protocol-ws ≈0.25 MB.
  `wasm-opt -Oz` (binaryen) typically shaves another 20-30% and slots in
  after wasm-bindgen in the `just wasm` recipe if it's ever worth
  installing.
- The size is acceptable because only /diary loads it (one admin, installed
  PWA, immutable-cached per deploy). Do not load this module from any
  public page.

## The surrealdb-core wasm patch (temporary, load-bearing)

surrealdb-core 3.2.3 panics at runtime on wasm32-unknown-unknown: most of
the crate migrated to `web_time` for the browser, but three call sites
still reach `tokio::time::Instant::now()` and std's unimplemented
monotonic clock — `RuntimeError: unreachable`. `kvs/ds.rs` (the
`check_version` retry on every datastore open) kills the engine before the
first query; the TIMEOUT query operator dies when used; and
`kvs/tasklease.rs` (`tokio::time::sleep` takes an Instant internally)
kills every spawned background task — index compaction, event processing,
tombstone reclaim — with a console exception per task on each page load.
This is the unresolved half of upstream issue #6711; the official
`@surrealdb/wasm` package dodges it only by pinning an older core.

The fix here is deliberately shaped like the upstream PR it should become:
`deploy/surrealdb-core-wasm-time.patch` swaps those three files onto the
crate's own established `wasmtimer` pattern (see its `sleep` call sites),
behind `cfg(target_family = "wasm")` — native code is byte-identical.
(`dbs/executor.rs` has two more `tokio::time::timeout` calls, but both are
gated on a datastore `transaction_timeout` this build never configures;
they belong in the upstream PR, not in this minimal patch.)

Containment is structural, not procedural: `crates/diary-worker` is its OWN
cargo workspace, excluded from the repo root's, and the `[patch.crates-io]`
lives in the worker's manifest. The server workspace cannot see it — the
site always builds pristine crates.io code, and no root-workspace command
touches the vendor dir. `scripts/vendor-surrealdb-core.sh` (run by
`just wasm` and the Dockerfile) materializes `vendor/surrealdb-core`
(gitignored) from the sha256-verified crates.io tarball plus the patch; the
repo commits only the ~40-line patch file. When a surrealdb release fixes
#6711 fully: bump the workspace pin, delete the `[patch]` block, the
script, the patch file, and this section.

## Direct sync (flag-off by default)

With `DIARY_SYNC_JWT_PUBLIC_KEY` + `DIARY_SYNC_JWT_PRIVATE_KEY` +
`DIARY_DIRECT_SYNC_ENDPOINT` set (railway-deploy.md), the sync pass skips
the site's endpoints entirely: the worker POSTs `/api/diary/token` (admin
cookie; the app server's ONLY remaining role), opens a fresh short-lived
WEBSOCKET to the endpoint, `authenticate()`s, verifies the `$access` canary,
and then `sync::DirectRemote` pushes with the same `store::save_entry`
probe the server runs and pulls the same snapshot read — one algorithm, two
engines, natively tested store-to-store. Any failure arming the pass falls
back to the HTTP endpoints, so a half-configured flag never silences sync.

Load-bearing findings (probed on 3.2.3, tests + canaries pin them):

- The access method MUST be `TYPE RECORD WITH JWT` and the token MUST carry
  an `id` claim (`diary_device:admin`). Plain `TYPE JWT` sessions get a
  database-level Viewer role that reads EVERY table regardless of
  PERMISSIONS.
- The endpoint MUST be `ws://`/`wss://`. The stateless-http engine's
  `authenticate()` does not stick on server 3.2.3 — every later request
  arrives anonymous.
- SurrealDB filters permission-denied reads to EMPTY results instead of
  erroring, so a silently-deauthed session pulls "an empty diary". Three
  layers keep that from touching the mirror: the setup canary
  (`RETURN $access`), the wipe guard (an empty snapshot never deletes a
  populated mirror), and per-pass fresh tokens (15-minute TTL).
- `jsonwebtoken` requires the private key as PKCS#8 PEM (`openssl pkcs8
  -topk8`), not SEC1 "EC PRIVATE KEY".

## What remains (all optional)

The original roadmap — direct client→SurrealDB, local-first reads, SSR in
the service worker — is fully shipped (2026-08-04, one branch, phased).
Leftover niceties, none load-bearing:

1. **`wasm-opt -Oz`** (binaryen) typically shaves 20–30% off the module;
   slots in after wasm-bindgen in `just wasm` if ever worth installing.
2. **Upstream topcoat PR** cfg-gating the `+ Send` bounds on
   `Component::render` / `PageRenderFn` for wasm32 — would retire the
   oneshot bridge in `diary-worker::ssr`, nothing else.
3. **Pull as `CHANGEFEED` + `SHOW CHANGES`** if the full-snapshot pull
   ever gets heavy (it is a personal diary; it will not soon). Changefeeds
   must stay server-side only — the wasm engine's changefeed GC has an
   open upstream issue (#6311).
4. **Live queries over the direct websocket** for real-time cross-device
   updates, if two devices ever compose at once in practice.
