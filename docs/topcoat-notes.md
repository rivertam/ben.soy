# Topcoat 0.5.0 crib sheet

(Written against 0.1.3, upgraded in place since; verified on 0.5.0. Releases
0.2–0.4 changed server and render behavior, not signatures: 0.2.0 added
default response compression and graceful shutdown, 0.3.1 escaped signal
hydration comments, 0.4.0 swapped the float formatter and taught the dev
watcher the whole package directory. 0.5.0 (2026-07-27) is the first
signature-breaking release — error constructors moved to
`topcoat::router::error`, session `Config` became `SessionConfig`, layouts
take a rendered `Result<View>` instead of a `Slot` future, boolean attribute
values got real omit-on-false semantics, `AssetConfig::hosted_at` swapped
arg order (unused here) — every section below reflects the 0.5.0 shape, and
the wasm door 0.5.0 opened is a new section at the end.)

Ground truth (read these, don't guess APIs):

- Vendored crate sources: `~/.cargo/registry/src/index.crates.io-*/topcoat-*-0.5.0/`
- Repo (examples!): `https://github.com/tokio-rs/topcoat` — release tags are
  plain again (`v0.5.0`; the 0.4.0-era per-crate `topcoat-v*` scheme is
  gone): `git clone --depth 1 --branch v0.5.0 https://github.com/tokio-rs/topcoat`
  — examples include `hello-world`, `module-router`, `path-query-params`,
  `runtime`, `shard`, `tailwind`, `font`, `asset`, `htmx`, `alpine-ajax`,
  `session`, `ui`, and (new in 0.5.0) `datastar`, `sse`, `websocket`,
  `mail`, `procedure`, `toasty-todo`
- Changelogs: `crates/<crate>/CHANGELOG.md` in the repo (no GitHub releases)
- docs.rs: https://docs.rs/topcoat/0.5.0

## Pages, layouts, routes (explicit paths + discover)

```rust
use topcoat::{Result, context::Cx,
    router::{Router, RouterBuilderDiscoverExt, layout, page, query_params},
    view::{View, component, view}};

#[tokio::main]
async fn main() {
    topcoat::start(
        Router::builder()
            .assets(topcoat::asset::AssetBundle::load().unwrap())
            .discover()
            .build(),
    ).await.unwrap();
}

#[layout("/")]                       // wraps every page under the prefix
async fn shell(slot: Result<View>) -> Result {   // 0.5.0: rendered, not a future
    view! { <!DOCTYPE html> <html> <head>topcoat::dev::script()</head>
        <body>(slot?)</body> </html> }
}

#[page("/thoughts")]
async fn thoughts() -> Result { view! { <h1>"…"</h1> } }
```

- `#[route(GET "/api/x")]` for non-page endpoints; `Json<T>`, `Form<T>`
  extractors live in `topcoat::router::content` (no longer re-exported at
  the router root as of 0.5.0).
- Errors: `topcoat::router::error::{not_found, bad_request, redirect,
  redirect_permanent, see_other, unauthorized, forbidden, …}` — 0.5.0 moved
  them off the router root (#183); usage unchanged:
  `Err(redirect("/thoughts").into())`. `redirect` = 307,
  `redirect_permanent` = 308, `see_other` = 303 (same codes as 0.4.0).
  `#[query_params(error = redirect("?"))]` needs no import — the macro
  resolves the name itself.

## Query params

```rust
#[query_params(error = redirect("?"))]   // bad parse → redirect w/ cleared qs
struct FlightQuery { from: Option<String>, to: Option<String>,
                     cabin: Option<String>, oneway: Option<String>, view: Option<String> }

#[page("/thoughts/how-bad-are-planes")]
async fn planes(cx: &Cx) -> Result {
    let q = query_params::<FlightQuery>(cx)?; …
}
```

## view! syntax rules

- Text is quoted: `"Hello"`. Interpolate with parens: `(expr)` — escaped.
- Components are called like functions inside markup: `hello(name: "World")`.
- Control flow: `for item in items { <div>(item)</div> }`,
  `if cond { … } else { … }` directly inside markup.
- Attributes: `href=(expr)`, `class="static"`, `class=(class!{…})`,
  `style=(format!(…))`.
- Boolean attributes (0.5.0, #179): a Rust `bool` value — `disabled=(flag)`
  — renders `disabled=""` when true and omits the attribute when false;
  `Option` values omit on `None`. Static string spellings (`hidden=""`,
  `required=""`) render verbatim as before, and a reactive `:hidden=$(…)`
  serializes an initially-true state as `hidden=""` where 0.4.0 wrote
  `hidden="true"` — byte-different, same meaning to the browser (verified
  against prod during the upgrade).
- `view::class!` / `view::attributes!` build dynamic class lists / attr sets.
- Raw trusted HTML: `topcoat::view::Unescaped::new_unchecked(svg_string)`
  interpolated with `(…)` — ONLY for markup we generate (e.g. qrcode SVG).
- SVG: `view!` handles `<svg>` elements; `topcoat::view::svg` has helpers
  (e.g. `ViewBox`).
- A `View` value (what components return inside `Result`) interpolates
  unescaped — that's how `(slot.await?)` works.
- An interpolated `f64` renders through std's `Display` (0.4.0 dropped the
  zmij formatter), so whole numbers lose their `.0`: `cy=(20.0)` emits
  `cy="20"`, and huge/tiny values expand instead of going exponential
  (`1e21` → `1000000000000000000000`). Harmless in SVG, but it means raw
  float interpolation is not byte-stable across topcoat versions — anything
  whose exact text matters goes through a formatter (`f2()` in
  `planes/instruments.rs`, the Intl-mirroring helpers elsewhere).

## Client interactivity (runtime feature)

Needs `topcoat::runtime::script()` in `<head>` (next to `dev::script()`).

```rust
view! {
    signal open = false;                       // declared inside view!
    <button @click=$(|_e| open.set(!open.get()))>"toggle"</button>
    <div :hidden=$(!open.get())>"…"</div>      // :attr = reactive binding
    $(if open.get() { "on" } else { "off" })   // reactive text
}
```

`$(…)` is real Rust, type-checked, transpiled to JS — keep it simple
(signal get/set, string ops, arithmetic, if/else). `e: topcoat::runtime::Event`
has `e.target.value`. 0.5.0 added utility methods on primitive-typed signals
(#214), but surrogates are still scalars only — bool/f64/string/Option/Result,
no collections, no reactive list rendering.

Each signal's initial value ships as an HTML comment
(`<!-- ::topcoat::signal({…}) -->`) for the client to hydrate. Those payloads
went unescaped until 0.3.1, so a signal seeded from untrusted input could
close the comment and inject markup. This site was never exposed — every
signal starts from a literal, an allowlisted string, or a server-computed
number — and should stay that way.

## Shards (server re-render on client change)

```rust
#[component]
async fn combobox() -> Result {
    view! {
        signal input = String::new();
        <input :value=$(input.get()) @input=$(|e: Event| input.set(e.target.value))>
        suggestions(input: $(input.get()))     // shard called w/ $() arg
    }
}

#[shard]                                        // topcoat::runtime::shard
async fn suggestions(cx: &Cx, input: String) -> Result {
    let hits = search(&input);                  // SERVER-side work
    view! { for h in hits { <div>(h)</div> } }
}
```

Shard re-renders server-side whenever a `$()` argument changes; HTML is
swapped in place. This is how the airport combobox and the cut-chips chart
stay server-computed.

## Tailwind

- `build.rs` (build-dep: `topcoat = { version = "0.5.0", default-features = false, features = ["tailwind"] }`):
  ```rust
  fn main() {
      println!("cargo:rerun-if-changed=styles/input.css");
      topcoat::tailwind::BuildConfig::new().input("styles/input.css").render().unwrap();
  }
  ```
  Downloads the standalone Tailwind v4 CLI on first build (network).
- `styles/input.css` starts with `@import "tailwindcss";` then `@theme { … }`
  tokens and any plain CSS (the ported how-bad styles live here, scoped under
  `.dispatch`). Tailwind v4: theme via CSS, no config file.
- Head: `<link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>`.
- Source scan honors `.gitignore` from the package root.

## Fonts (font-fontsource feature)

```rust
use topcoat::font::{Font, fontsource::fontsource_font};
const ZILLA_SLAB: Font = fontsource_font!(ZILLA_SLAB, host: Asset); // downloads + self-hosts
// head: topcoat::font::link(font: ZILLA_SLAB)
// css:  font-family via ZILLA_SLAB.family() or hardcode "Zilla Slab" in @theme
```

## Assets

`asset!("./file.png")` (relative to the source file) → content-hashed URL.
Router needs `.assets(AssetBundle::load().unwrap())`.

## Running (VERIFIED recipe for this repo)

```sh
export PATH="$HOME/.cargo/bin:$PATH"   # topcoat CLI lives here
cargo build                            # also runs tailwind via build.rs
topcoat asset bundle --bin benjisponge # extracts embedded assets → target/assets/
PORT=4610 ./target/debug/benjisponge     # serve (defaults 127.0.0.1:3000)
```

`AssetBundle::load()` panics without the `topcoat asset bundle` step (it
searches `target/assets`). `--bin benjisponge` is required since the crate
grew a second binary (spire_sync) — the bundler refuses to guess. Release:
`cargo build -r && topcoat asset bundle -r --bin benjisponge`.
`topcoat dev` = watch mode (build+bundle+serve), not script-friendly.
`topcoat::dev::script()` = live-reload in dev; harmless in release.
`PORT=4670 topcoat dev` — app keeps the PORT; the reload broadcast
server takes its own ephemeral port (TOPCOAT_DEV_URL). Verified: save a
src file → rebuild → open tabs reload themselves in a few seconds.
Since 0.3.1 the watcher covers the whole package directory minus
`.gitignore`d paths, hidden entries, and `target/` — `styles/`, `data/`,
`build.rs`, and `Cargo.toml` all trigger a rebuild now (verified on 0.4.0
by appending a comment to `styles/site.css`). The CLI is installed
separately from the crate, so `cargo install topcoat-cli --version <v>
--locked` and restart `topcoat dev` after bumping the dependency —
`deploy/Dockerfile` pins the same version for the release build.

## Gotchas (several LEARNED THE HARD WAY here)

- Path params arrive percent-DECODED (`RawPathParams::from_pairs` in
  topcoat-router), and `redirect()`/`see_other()` build the `Location`
  header with `HeaderValue::try_from(...).expect(...)` — a param containing
  decoded control bytes (`/x/%0A`) PANICS the connection task. Never put a
  raw path param in a redirect target: validate its exact shape first
  (diary) or re-encode it (`workout_url` runs `urlencode`).
- Component invocations (including `topcoat::dev::script()`,
  `topcoat::font::link(…)`) only work inside `view!` **within a
  `#[page]`/`#[component]`/`#[layout]`/`#[shard]` fn** — the expansion needs a
  hidden `__cx`. A plain `async fn` containing `view!` cannot call components.
- Page-shell pattern used in this repo (`src/components/chrome.rs`): `#[component] shell(title: &str, body: View)`;
  pages do `let body = view! { … }?;` then `view! { shell(title: "…", body: body) }`.
- Unquoted text in `view!` is a compile error — every literal string is quoted.
- `#[component]`/`#[shard]` functions are `async` and return `topcoat::Result`.
- Component props: `&str` props are proven; if a reference/struct prop fails
  to compile, fall back to owned values (String/f64/cloned struct).
- `signal` declarations live inside `view! {}`, before markup.
- Keep `$()` expressions primitive; complex logic belongs in the shard/server.
- Styles: `styles/input.css` `@import`s per-section files
  (`site.css`, `planes-form.css`, `planes-receipt.css`, `planes-charts.css`);
  `build.rs` watches the whole `styles/` dir. Edit only your own section file.
- rustc ≥1.95 required (we're on 1.97).

## Wasm (new in 0.5.0)

- topcoat-router's hyper/tokio server now sits behind an opt-in `serve`
  feature, so `topcoat = { default-features = false, features = ["router",
  "view", "discover"] }` compiles on wasm32-unknown-unknown — `#[page]`,
  inventory discovery, and `Router::handle(req)` (socket-free dispatch)
  included. Probe-verified 2026-08-04.
- The catch: page/component render futures are `+ Send` with no wasm cfg
  (`topcoat_view::Component::render`, router `PageRenderFn`), and browser
  interop futures are `!Send` — a page fn cannot directly await an indxdb
  query or JsFuture. Either bounce such work through
  `wasm_bindgen_futures::spawn_local` + a oneshot channel (the receiver half
  is `Send`), or upstream a cfg-gated bound. The diary's plan for this is
  roadmap item 3 in `docs/diary-sync.md`.
