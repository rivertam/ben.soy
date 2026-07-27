# Golden fixtures — Rust data-layer contract

Captured from production 2026-07-23 by `capture.sh`, while the TypeScript
Worker (`deploy/src/fitness.ts`, `deploy/src/spire.ts`) still owned the data
path. There is no Worker test suite; **these fixtures are the API contract**
for the Rust port.

State at capture: fitness version 135, spire version 11; 360 workouts /
5,561 sets / 221 exercises / 638 tags / 4,113 set_records / 199 spire runs
(matches the audited baseline in `docs/fitness.md`).

## Layout

- `api/manifest.tsv` — `name  status  method  path?query` for every capture.
- `api/<name>.json` — response body, byte-exact as served.
- `api/<name>.headers` — full response headers for representative cases.
- `d1/workout_triples.json` — all 360 `(started_at_utc, started_at_local,
  eastern_offset_minutes, duration_seconds, id)` rows; the validation corpus
  for the Rust Eastern-projection module (`benjisponge::eastern`).
- `d1/spire_rows.json` — all spire runs, every column except `raw`.
- `d1/meta_versions.json`, `d1/counts.json` — provenance.

Wrangler D1 dumps are wrapped: parse `[0].results`.

## Golden-diff mask policy

When diffing the Rust implementation's responses against `api/*`, the
following MUST be masked (expected to differ by design); **everything else
must diff clean, byte-for-byte where possible**:

1. **`records` arrays on every set** — production values were imported from
   Lyfta's CSV columns; the Rust port derives historical PR-at-the-time
   records instead. Different by design (Ben, 2026-07-22).
2. **Every capture whose query includes `has_record=`** — the filter now
   evaluates derived records, so `total_sets`/`total_workouts`/pagination
   and the returned workouts may all legitimately differ
   (`sets_has_record`, `sets_has_record_f`).
3. **`version` fields** — the database counter restarts on reseed; compare
   presence/type, not value. (`/api/fitness/ids` has no version field —
   that absence IS contract.)
4. **Spire re-derived fields** — if prod spire data is replayed through
   `spire_sync` from dumped `raw` files, fields the CLI derives (e.g.
   `date`) reflect today's CLI code; `added_at` is not served by the API.
   The multi-game API also adds `game` to each run and scopes `/api/spire/ids`
   entries as `game:id`; frozen Worker fixtures predate both additions.
5. **Volatile headers** — `date`, `cf-ray`, `server`, `alt-svc`, timing
   headers. Contract headers that must MATCH: `content-type`,
   `cache-control: no-store`, `access-control-allow-origin: *` on public
   GET reads (absent on import/POST responses; spire responses never had
   CORS).
6. **`/api/*/ids` array order** — `SELECT id FROM ...` with no ordering;
   storage order differs between D1 and the current store. Compare as sets (the
   sync CLIs already do).

Statuses in `manifest.tsv` must match exactly, including the 400/401/404
split and the exact `{"error": "..."}` message bodies (note
`err_perpage15` vs `err_perpage50` — two different messages for the same
parameter, both contract).

## Re-capturing

These fixtures are frozen provenance from the retired Worker path.
`capture.sh` remains only to document how they were collected; do not
overwrite them from the current production service.
