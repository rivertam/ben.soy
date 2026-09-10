# Shared thought-page calculations

`thoughts-core` owns the pure calculations and decimal formatting used by the
server and public calculator enhancements. `thoughts-worker` is a thin
wasm-bindgen adapter; JavaScript keeps DOM updates, controls, focus, and URLs.
The crop receipt and the planes number formatter use decimal-string rounding
half away from zero, matching `Intl.NumberFormat` even at ties such as 1.005.

`just thoughts-wasm` emits an ES module and Wasm pair into ignored `wasm-dist/`.
The wasm-bindgen CLI must match the pinned 0.2.126. Development, build, release,
Docker, and CI build the pair. `just test-browser` runs executable adapter tests
against the generated Wasm, including comparisons with the JavaScript Intl
implementation. `just check` runs the native workspace tests.

`/thoughts-core.js` is a no-cache ES loader. It imports the generated glue and
Wasm using one hash covering both files. Those byte routes are immutable only
for the exact current hash, so deploy skew cannot pin a mismatched pair in the
browser cache. Files are read by stamp at request time; rebuilding Wasm does
not require rebuilding the server. With missing artifacts or unavailable Wasm,
the ordinary server-rendered GET forms remain usable.
