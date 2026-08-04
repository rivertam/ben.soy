# The diary queue in Rust (client-side SurrealDB over wasm)

The /diary offline write queue is written in Rust on both sides of the wire.
This is stage 1 of a larger idea — Remix-style loader/clientLoader
isomorphism for a topcoat + SurrealDB app — and the roadmap at the bottom is
the rest of it.

## Shape

- `crates/diary-core` — the whole queue, shared. `contract` is the replay
  protocol (`WireEntry`, the composition-second window, body normalization,
  response classification); `outbox` is the queue itself, written against
  the SAME `Surreal<Any>` handle `src/data.rs` uses for the real database.
  `cargo test -p diary-core` runs the outbox against `mem://`; the phone runs
  the identical code against `indxdb://diary` (SurrealDB's IndexedDB engine).
  Nothing in the crate knows which engine it is on — that is the point.
- `crates/diary-worker` — the wasm skin: five `#[wasm_bindgen]` exports
  (`diary_enqueue/snapshot/discard/import/flush`) plus the one genuinely
  browser-shaped piece, the `fetch` transport injected into
  `diary_core::outbox::flush`. Resolved off `js_sys::global()`, never
  `window`, because it runs in the service worker and the page.
  Deliberately its own cargo workspace with its own lockfile, EXCLUDED
  from the root workspace — see the patch section below. That means
  `just check` and CI never touch it: a diary-core API change that breaks
  the worker surfaces at `just wasm` or the Docker wasm-builder stage, not
  in check. (The crate root is also `cfg(target_arch = "wasm32")`-gated so
  a stray native build compiles an empty crate rather than erroring.)
- `src/app/diary.rs` — the server half consumes `diary_core::contract` for
  parsing and validation. One definition of the protocol, compiled into both
  binaries; a drift is now a type error, not a silent half-parse.
- `src/app/diary_sync.rs` — serves `wasm-dist/` at three stable routes (see
  Serving below).
- `src/app/diary/sw.js`, `diary.js` — reduced to browser glue: caching,
  Web Lock, Background Sync, BroadcastChannel, DOM. The queue policy they
  used to implement in duplicate lives in `diary-core::outbox` now.

## Flush semantics (unchanged, now tested)

Oldest-first by enqueue time; stop on auth (401/404) or retryable trouble
(network, 403, 5xx, captive-portal 200) so composition order survives;
permanent rejections (400/409/413/415/422) mark the entry failed and keep
its text for manual copy. The server's same-second+same-body dedupe remains
the real idempotency guarantee. All of it is pinned by native tests in
`diary-core` — the policy that used to live only in sw.js is exercised by
`cargo test` on every check.

The `FlushReport` carries `saved_entries` — for each drained entry, its
local qid plus the id/written_at the server's response assigned (collision
probes can bump the second, so the server's identity is the truth). That is
what lets the page be optimistic: a sent message renders synchronously in
the submit handler, survives every repaint as a draft or queued bubble
(reconciled by qid), and flips to a delivered message with a real permalink
when the report lands — no post-flush reload, which is also why the worker
refreshes the cached /diary copy itself after a saving flush. On-their-way
bubbles wear their final look (same style, same stamp shape) so delivery
rewrites nothing visible; the dashed queued styling and "will sync" label
appear only when a flush report says the queue is actually blocked, and
"failed" only when the server rejected the entry. Two mid-flush
subtleties the page covers: the worker deletes each saved entry from the
store immediately but reports once at the end, so a pending entry that
vanishes from a snapshot un-reported rides a provisional bucket instead of
blinking away; and a report for an entry the server-rendered HTML already
shows (a page opened mid-flush) retires bubbles by qid without drawing the
message twice.

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
  (`opt-level = "z"`, fat LTO, `panic = "abort"`). Current output ≈18 MB
  raw / ≈4.8 MB gzipped — the embedded SurrealDB engine is the floor.
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

## Roadmap (the rest of the idea)

1. **Direct client → SurrealDB for the diary** — `DEFINE ACCESS ... TYPE
   JWT` on the server database, per-table `PERMISSIONS` for diary tables, a
   cookie-gated endpoint minting short-lived tokens, `/rpc` exposed through
   the tunnel. `outbox::open` already takes any endpoint; the flush
   transport becomes a direct `Surreal<Any>` write instead of a fetch.
2. **Local-first reads** — mirror `diary_entries` into the local store,
   render from it, pull server changes via `CHANGEFEED` + `SHOW CHANGES`
   (changefeeds defined server-side only; the wasm engine's changefeed GC
   has an open upstream issue), push via this outbox.
3. **SSR in the service worker** — topcoat 0.5.0 (2026-07-27) shipped the
   feature split this item was blocked on: hyper/tokio now sit behind an
   opt-in `serve` feature, and `topcoat = { default-features = false,
   features = ["router", "view", "discover"] }` compiles on wasm32 —
   `#[page]`, discovery, and `Router::handle(req)` included (probe-verified;
   the site runs 0.5.0 as of this branch). One wall left: page/component
   render futures are `+ Send` with no wasm cfg
   (`topcoat_view::Component::render`, router `PageRenderFn`), and browser
   interop futures are `!Send` — the exact fragility `outbox::flush`'s
   signature dodges. So a worker page fn can't await indxdb queries
   directly; either bounce local-store reads through
   `wasm_bindgen_futures::spawn_local` + a oneshot channel (the receiver is
   `Send`), or land the small upstream PR cfg-gating those bounds on the
   single-threaded target. Then the worker can answer `GET /diary` offline
   by running the same page fn against the local store.
