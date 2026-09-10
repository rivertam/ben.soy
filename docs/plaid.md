# Plaid

`/admin/plaid` is the owner’s visual cloth builder. Palette and band controls,
the JSON editor, and seeded generation all converge on the same validated
`plaid::Pattern`. CSS, swatches, and fitness PNGs derive from that value.
The browser adapter only edits controls and formats band notation; parsing,
generation, contrast decisions, and drawing stay in Rust. No worker or new
Wasm build is involved.

## Definition

```json
{
  "version": 1,
  "palette": {
    "K": "#192b24",
    "B": "#071426",
    "R": "#7e252d",
    "Y": "#e2c168"
  },
  "warp": "K/24 B24 R4 Y/2",
  "repeat_px": 96,
  "rotation_deg": 10
}
```

Bands use the [Scottish Register’s threadcount notation](https://www.tartanregister.gov.uk/threadcount).
Letters name palette colors; positive whole numbers count threads. Mirrored
sequences put `/` on both end bands. Pivot counts are **full stripe widths**:
`K/24 B8 Y/2` expands to `K24 B8 Y2 B8`, then repeats. Do not duplicate the
pivots. Repeating sequences use `...K24 B8 Y2...` and restart directly.

`warp` describes vertical bands. Omit `weft` to use the same sequence
horizontally, or supply an independent threadcount string. `repeat_px` sets
the width of the expanded warp repeat; both axes use the resulting thread
size, so an independent weft may have a different repeat height.

Limits: 2–16 palette colors, unique 1–3 ASCII-letter codes, six-digit hex
colors, 2–32 bands per input axis, 1–512 threads per band, at most 4,096
expanded threads per axis, 24–320 px repeat width, and −45–45° rotation.
Unsupported versions, unknown fields, and undefined colors are rejected.
Canonicalization uppercases codes, lowercases hex, normalizes spacing, and
omits a redundant weft. A SHA-256 of the canonical document identifies it.

## Generation and rendering

Generator version 1 uses `rand`'s `ChaCha8Rng`, seeded by hashing a 1–128
character string once with SHA-256. `Distribution<Spec> for StandardUniform`
supports `rng.random::<Spec>()` with any `rand` RNG. The custom distribution
keeps stripe codes aligned with the generated palette; it shares its recipes
with the partial rerolls. Random plaid chooses a palette, stripe rhythm,
scale, and a uniformly sampled whole-degree rotation from 0° through 15°;
random colors preserves geometry; random stripes
preserves palette, scale, rotation, and whether the axes are independent.
After release, change the generator version when changing its RNG, sampling,
or recipes.
A fixed seed fixture guards against accidental replay changes when
dependencies are updated. Stored cloth
always contains the actual definition, so it does not depend on future
generator behavior.

The random buttons pick a fresh seed. Repeat seed uses the visible seed and
the retained inputs from the last generation; editing that seed allows exact
named experiments. Before any generation, Repeat seed makes a full plaid.
Copy the pattern text to keep a definition; there is one saved current plaid,
not a pattern library or an automatic rotation schedule.

Crossings blend the two yarn colors. Fine diagonal twill repeats independently
of the bands, preventing texture seams at odd thread totals. CSS gradients
repeat without clipping rotated tiles; SVG uses independently repeating
vector gradients. The raw cloth preview and menu swatch show the original colors.

The display finish chooses dark or light ink and the least neutral backing
needed for contrast across every crossing and the texture extrema. All text
roles target at least 4.5:1 on the final background and solid card surfaces.
The page and Thursday social card share that finish. Draft CSS is scoped to
the preview and cannot recolor the editor’s surrounding page.
The sample PNG uses the same drawing code at 600×300 for responsive editing;
published fitness social cards remain 1200×600.

## Publication and caching

Admin-only endpoints, all `no-store`:

- `GET /admin/plaid`: current definition and editor.
- `POST /admin/plaid/preview`: `{spec}` → canonical document, band models,
  scoped preview CSS, and a sample PNG data URL. No database writes.
- `POST /admin/plaid/generate`: `{spec,seed,generator_version,mode}`, with
  mode `all`, `colors`, or `stripes` → the same preview response. No writes.
- `POST /admin/plaid`: `{spec,expected_revision}` → preview plus the saved
  `revision` and `updated_at`. This is the only publishing operation.

Every POST independently checks exact admin identity, positive same-origin
evidence, JSON content type, and a 32 KiB body limit. Publication atomically
checks the prior revision and writes `plaid_settings:current` with the native
document, incremented revision, fingerprint, and timestamp. A stale editor
gets 409, validation gets 422, and database failure gets 503. Mechanical
JSON/body/content-type failures use 400/413/415. The browser keeps its draft
on failure; copy it before resetting after a conflict.

The singleton is separate from fitness history and fitness snapshot versions.
Schema reconciliation adds its fields without modifying a saved definition.
An absent record renders the built-in forest/navy/oxblood/gold cloth at 10°;
the first explicit save creates revision 1. Public reads cache for two seconds,
retaining the last valid cloth on failure with a five-second retry cooldown.
Admin reads bypass that debounce, and a save updates the process cache at once.

The shell links stable `/plaid/current.css` after bundled CSS. It returns
`text/css`, `no-cache`, and a content ETag so day-cached HTML can use newly saved
cloth. Its fallback stays legible if the stylesheet cannot load. Open pages
pick up changes on reload; the editor refreshes its stylesheet after saving.

Existing Thursday rules stay intact: visitor-local Thursday for the site,
including its same-day theme override, and the workout’s Eastern **start** date
for fitness PNGs. All Thursday workouts use the current plaid, including old
workouts. Non-Thursday cards retain their grid and do not read the plaid store.
Thursday image URLs add `p=<fingerprint>` to the existing `v`/`r` fields. Only
an exact match gets immutable caching; stale/missing queries and unavailable
plaid-store responses revalidate. Already cached third-party previews may
continue showing the old image until that service fetches new metadata.

## Verification

`just check` covers the parser, generator, contrast calculation, native
SurrealDB persistence and conflict races, admin routes, and PNG cache rules.
`just build` and starting the bundled app also check asset/route registration.
Browser checks should exercise invalid text, rapid edits, reorder/remove,
independent axes, named seeds, publishing and resetting, failed saves, and
desktop/mobile layouts with light, dark, and mixed black/white cloth.
