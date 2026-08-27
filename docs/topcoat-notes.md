# Topcoat 0.6.2 crib sheet

(Written against 0.1.3, upgraded in place since; verified on 0.6.2. The
0.5.0 migration moved router errors, changed layouts to rendered
`Result<View>` slots, fixed boolean attributes, and opened the serve-less wasm
build. 0.6.0 (2026-08-17) is another signature-breaking release: request and
response helpers moved into dedicated modules, `Cx` became immutable and
scoped, path parameters became `path_param!` declarations, handler structs
became traits, component rendering became concurrent, unmatched requests
stopped running path-scoped layers/layouts, request origins and buffered body
sizes gained secure defaults, and asset bundles moved beside their exact
profile executable. 0.6.1 and 0.6.2 (2026-08-18) are compatible fixes; every
section below reflects 0.6.2.)

Ground truth (read these, don't guess APIs):

- Vendored crate sources: `~/.cargo/registry/src/index.crates.io-*/topcoat-*-0.6.2/`
- Repo (examples!): `https://github.com/tokio-rs/topcoat` — release tags are
  plain (`v0.6.2`):
  `git clone --depth 1 --branch v0.6.2 https://github.com/tokio-rs/topcoat`
  — examples include `hello-world`, `module-router`, `path-query-params`,
  `runtime`, `shard`, `tailwind`, `font`, `asset`, `htmx`, `alpine-ajax`,
  `session`, `ui`, and (new in 0.5.0) `datastar`, `sse`, `websocket`,
  `mail`, `procedure`, `toasty-todo`
- Migration notes: https://github.com/tokio-rs/topcoat/releases/tag/v0.6.0;
  patch notes: `v0.6.1` and `v0.6.2`; per-crate changelogs live at
  `crates/<crate>/CHANGELOG.md` in the repo
- docs.rs: https://docs.rs/topcoat/0.6.2

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
async fn shell(slot: Result<View>) -> Result {   // rendered, not a future
    view! { <!DOCTYPE html> <html> <head>topcoat::dev::script()</head>
        <body>(slot?)</body> </html> }
}

#[page("/thoughts")]
async fn thoughts() -> Result { view! { <h1>"…"</h1> } }
```

- `#[route(GET "/api/x")]` for non-page endpoints; `Json<T>`, `Form<T>`
  extractors live in `topcoat::router::content` (no longer re-exported at
  the router root as of 0.5.0).
- Low-level request values/helpers live in `topcoat::router::request`
  (`Request`, `Bytes`, `headers`, `uri`); response types/traits live in
  `topcoat::router::response` (`Response`, `IntoResponse`). `Body`, HTTP
  header/status types, `to_bytes`, and the routing macros remain at the router
  root.
- Errors: `topcoat::router::error::{not_found, bad_request, redirect,
  redirect_permanent, see_other, unauthorized, forbidden, …}` — 0.5.0 moved
  them off the router root (#183); usage unchanged:
  `Err(redirect("/thoughts").into())`. `redirect` = 307,
  `redirect_permanent` = 308, `see_other` = 303 (same codes as 0.4.0).
  `#[query_params(error = redirect("?"))]` needs no import — the macro
  resolves the name itself.

## Path and query params

```rust
path_param!(post_slug);                 // generates `PostSlug`

#[query_params(error = redirect("?"))]   // bad parse → redirect w/ cleared qs
struct FlightQuery { from: Option<String>, to: Option<String>,
                     cabin: Option<String>, oneway: Option<String>, view: Option<String> }

#[page("/thoughts/how-bad-are-planes")]
async fn planes(cx: &Cx) -> Result {
    let q = query_params::<FlightQuery>(cx)?; …
}

#[page("/thoughts/{post_slug}")]
async fn post(cx: &Cx) -> Result {
    let slug: &str = path_param::<PostSlug>(cx); …
}
```

`#[path_param] struct Slug(str);` was removed in 0.6. Use
`path_param!(slug);`; typed parsing is `path_param!(id: u64, error =
bad_request)`, and catch-alls are `path_param!(*parts)`.

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
  unescaped — that's how `(slot?)` works in a layout.
- Components in one `view!` scope render concurrently as of 0.6. Do not borrow
  a value for surrounding markup and then move that same value into a
  component prop; pass a separate owned value/reference. Likewise pass owned
  temporaries directly (`href=(format!(...))`), never borrow them with
  `.as_str()` inside the view. The output order remains source order.
- `View::render` / `render_response` consume the view. Clone before the first
  render only when a test or adapter genuinely needs to render it twice.
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

- `build.rs` (build-dep: `topcoat = { version = "0.6.2", default-features = false, features = ["tailwind"] }`):
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
topcoat asset bundle --bin benjisponge # extracts embedded assets → target/debug/assets/
PORT=4610 ./target/debug/benjisponge     # serve (defaults 127.0.0.1:3000)
```

`AssetBundle::load()` panics without the `topcoat asset bundle` step (it
looks only for `assets/manifest.toml` beside the running executable). Debug
bundles therefore live at `target/debug/assets`; release bundles live at
`target/release/assets`, and the Docker image copies that directory beside
`/app/benjisponge`. `--bin benjisponge` is required since the crate has
multiple binaries — the bundler refuses to guess. Release: `cargo build -r &&
topcoat asset bundle -r --bin benjisponge`.
`topcoat dev` = watch mode (build+bundle+serve), not script-friendly.
`topcoat::dev::script()` = live-reload in dev; harmless in release.
`PORT=4670 topcoat dev` — app keeps the PORT; the reload broadcast
server takes its own ephemeral port (TOPCOAT_DEV_URL). Verified: save a
src file → rebuild → open tabs reload themselves in a few seconds.
Since 0.3.1 the watcher covers the whole package directory minus
`.gitignore`d paths, hidden entries, and `target/` — `styles/`, `data/`,
`build.rs`, and `Cargo.toml` all trigger a rebuild now (verified on 0.4.0
by appending a comment to `styles/site.css`). The CLI is installed
separately from the crate, and 0.6 requires it to match: `cargo install
topcoat-cli --version 0.6.2 --locked`. Restart `topcoat dev` after replacing
the CLI; `deploy/Dockerfile` pins the same version for the release build.

## Gotchas (several LEARNED THE HARD WAY here)

- Layers take `cx: &Cx`; `CxBuilder`, mutation, and `Cx::detach` are gone.
  Derive scoped request context with `let child = cx.with(value)` and pass
  `&child` to `next.run`. A `#[layer("/")]` wraps matched routes only in 0.6;
  this site's response layer is a manual pathless `LayerFn` so bare 404/405
  responses still receive its privacy headers.
- Every router now rejects state-changing cross-origin browser requests by
  default. Configure `OriginPolicy` only for a deliberate cross-origin write;
  requests from non-browser sync clients (no `Origin`/`Sec-Fetch-Site`) still
  pass. Buffered extractors default to a 2 MiB limit; raw `Body` handlers keep
  their explicit bounds here. `to_bytes` now returns `topcoat::Result<Bytes>`
  and classifies over-limit bodies as a renderable 413.
- `RouteFn`/`PageFn`/`LayerFn` are concrete implementations of handler traits,
  their `const_new` constructors are gone, and discovery inventories
  `&'static dyn Route`/`Page`/`Layer`. The post shortlink macro implements
  `Route` on a unit marker because its path must remain compile-time-derived.
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
- `#[component]` treats EVERY parameter as a prop except one literally named
  `cx: &Cx` — naming it `_cx` makes it a required prop (`missing required
  property …::_cx`). A component that never touches context can simply omit
  the parameter entirely; the macro supplies `__cx` regardless
  (diary-core's shared views do this).
- Each `asset!` invocation registers the file's serving route at discovery,
  and TWO invocations of the same file (or two `tailwind::stylesheet!()`
  calls — it is an `asset!` underneath) panic the router build with
  "duplicate route registered", which `just check` never runs. One `pub`
  const per asset, imported everywhere else (`chrome::SITE_CSS`,
  `diary::DIARY_JS` are the shared ones).
- Page-shell pattern used in this repo (`src/components/chrome.rs`): `#[component] shell(title: &str, body: View)`;
  pages do `let body = view! { … }?;` then `view! { shell(title: "…", body: body) }`.
- Unquoted text in `view!` is a compile error — every literal string is quoted.
- `#[component]`/`#[shard]` functions are `async` and return `topcoat::Result`.
- Component props: `&str` props are proven; if a reference/struct prop fails
  to compile, fall back to owned values (String/f64/cloned struct).
- Concurrent view futures make the site's expanded types deep enough to need
  `#![recursion_limit = "512"]` in `src/main.rs`; lowering it reproduces trait
  solver overflows in the lifting and crop-deaths pages.
- `signal` declarations live inside `view! {}`, before markup.
- Keep `$()` expressions primitive; complex logic belongs in the shard/server.
- Styles: `styles/input.css` `@import`s per-section files
  (`site.css`, `planes-form.css`, `planes-receipt.css`, `planes-charts.css`);
  `build.rs` watches the whole `styles/` dir. Edit only your own section file.
- rustc ≥1.95 required (we're on 1.97).

## Wasm (serve split introduced in 0.5.0; verified on 0.6.2)

- topcoat-router's hyper/tokio server now sits behind an opt-in `serve`
  feature, so `topcoat = { default-features = false, features = ["router",
  "view", "discover"] }` compiles on wasm32-unknown-unknown — `#[page]`,
  inventory discovery, and `Router::handle(req)` (socket-free dispatch)
  included. Probe-verified 2026-08-04.
- The catch: page/component render futures are `+ Send` with no wasm cfg
  (`topcoat_view::Component::render`, router `PageRenderFn`), and browser
  interop futures are `!Send` — a page fn cannot directly await an indxdb
  query or JsFuture. Two working answers, both SHIPPED in the diary worker
  (`crates/diary-worker/src/lib.rs::ssr`, live-verified in a real service
  worker): keep shared components PURE (zero awaits — a future that never
  suspends over `!Send` state is `Send` for free), and bounce actual store
  reads through `wasm_bindgen_futures::spawn_local` + a oneshot channel
  (the receiver half is `Send`). `Router::builder().discover().app_context(…)
  .build().handle(request)` then serves `#[page]`s from inside the worker;
  `Surreal<Any>` itself is Send+Sync as a HANDLE and registers fine as app
  context — only its query futures are `!Send`.
