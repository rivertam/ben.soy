# Fitness archive

Read this before changing `/lifting`, workout import, fitness API/schema, tags,
or local fitness startup. The exact API/filter/import contracts are in this
file under "API contract".

## Data flow

- Page, filter/query handling, API reader, and HTML rendering:
  `src/app/interests/lifting/`; styles are Tailwind utilities inline in those
  views (no section stylesheet). `/lifting` is the
  no-JavaScript landing view (daily volume heatmap plus the newest lift),
  `/lifting/log` is the filterable full archive, and
  `/lifting/YYYY-MM-DDTHH-MM-SS-04-00` (or `-05-00`) is a complete permanent
  workout page. Its timestamp is the `America/New_York` projection of the
  source instant; the explicit Eastern offset keeps same-date workouts and the
  repeated fall DST hour distinct without exposing importer IDs.
  `auto-filter.js` only debounces full-log form changes and navigates to the
  canonical GET URL; the Apply button remains the no-JavaScript path.
- `/lifting/log` also renders the volume heatmap, restricted to the sets the
  active filters admit. That calendar comes from
  `Snapshot::calendar_filtered` in-process (the same per-set predicate as the
  set log); the public `/api/fitness/calendar` endpoint deliberately still
  accepts no query parameters. Heatmap day cells carry the active filters
  (minus `from`/`to`/`page`) into their day links.
- Workout pages derive a muscle map (front/back SVG plus text lists) at
  render time: `muscles.rs` intersects each exercise's stored muscle tags
  with `PRIMARY_BY_MOVEMENT`, a static table mirroring which muscles each
  `fitness_sync::exercise_tags` movement rule treats as the movers; leftover
  muscle tags render as secondary. No rank is stored anywhere — like
  records, the split is derived, and the wire contract is unchanged (tags
  reach the page through `Snapshot::exercise_tag_map`, not JSON).
- Workout pages also render a plain-text share block (`share.rs` +
  `share.js`): a Strong-style set list ending in the workout's permanent
  URL, built from the request's Host/`x-forwarded-proto` like the planes
  receipt QR. The text lives in a readonly `<textarea>` — selectable
  without JavaScript, and on the em-dash layer's skip list
  (`src/emdash.rs`) so user-authored em dashes stay plain text;
  `share.js` only reveals the clipboard button.
- Public reads and the authenticated import:
  `src/app/interests/lifting/archive/` — `routes.rs` over the engine
  (filters, import validation, in-memory snapshot, store) and `db.rs`
  (SurrealDB). Records are derived in `archive/records.rs` at snapshot build —
  there is deliberately no records table and no records field in the import
  payload.
- Schema: `src/schema.surql` — five fitness tables:
  `workouts`, `exercises`, `exercise_tags`, `sets`, and `fitness_meta`.
- CSV parsing, stable IDs, taxonomy, chunking:
  `src/app/interests/lifting/fitness_sync.rs`.
- `just dev [port]` delegates to `scripts/dev.sh`: it starts the local
  SurrealDB container, then runs Topcoat with local-only sync tokens. The app
  applies the committed schema on its first data-backed connection. It never
  imports data.
- `just reset-fitness-local [csv]` runs while `just dev` is active. It
  truncates only the local fitness tables, resets the fitness version, and
  imports the CSV; local Spire tables in the shared database remain untouched.

## Source invariants

- CSV stays outside git. Audited baseline: 5,561 sets, 360 workouts, 221
  exercises, 2023-09-27 through 2026-07-21; 548 squat-type sets in 97 workouts.
- Strong's offset-less `Date` field is always UTC. Parse it as UTC, never as
  the machine's timezone or a local-naive timestamp.
- Keep the source instant as `started_at_utc`, then derive
  `started_at_local` and `eastern_offset_minutes` with the IANA
  `America/New_York` rules. This means Eastern time (EST *and* EDT), not a
  fixed EST offset. All public dates, calendar buckets, date/weekday/
  time-of-day filters, labels, and permanent lift URLs use that Eastern
  projection.
- Stable workout and set IDs remain derived from the raw UTC start timestamp
  (and the whole-workout ordinal for sets). Timezone conversion must never
  change identity, deduplication, or import ordering.
- Strong omits load and distance units. This archive assumes every imported
  load is pounds and persists `weight_unit='lbs'`; distance remains unitless.
- Strong labels effort `RIR/RPE`. On import, values below 6 are treated as RIR
  and converted with `RPE = 10 - RIR`; values at or above 6 are stored as RPE.
- Preserve apparent duplicate rows. Set identity is workout UTC start plus
  whole-workout ordinal; deduping or reordering changes IDs.
- Duration `0` or at least four hours is suspicious, not invalid. Preserve it.
- Load/distance are stored in thousandths and effort in hundredths. Keep
  integer scaling and explicit JSON nulls across importer, API, and UI.
- Records (`/lifting` badges) are derived from full set history when the
  in-memory snapshot is rebuilt, never stored or imported.

## API contract

Read endpoints are served from an immutable in-memory snapshot
(`src/app/interests/lifting/archive/snapshot.rs`), rebuilt when the fitness
version changes. Filter
semantics deliberately mirror the original Worker SQL (ASCII-only case
folding, byte-order sorts, NULL-excluding comparisons); the golden fixtures
under `tests/fixtures/api` are the contract.

Public reads are `Cache-Control: no-store` and include
`Access-Control-Allow-Origin: *`:

- `GET /api/fitness/sets` returns a workout-grouped page of matching sets:
  `{version,page,per_page,total_sets,total_workouts,workouts}`. Each workout is
  `{id,path,title,raw_title,started_at_local,ended_at_local,eastern_offset_minutes,end_eastern_offset_minutes,duration_seconds,duration_suspicious,notes,description,sets}`.
  `id` stays an opaque UTC-derived stable identifier; `path` is the canonical
  public path segment. Reader responses do not expose a `started_at_utc`
  field; all user-facing times are Eastern.
  each set is
  `{id,ordinal,exercise_name,raw_exercise_name,exercise_note,superset_id,weight_milli,weight_unit,reps,effort_hundredths,distance_milli,set_time_seconds,set_type,records}`;
  each record is `{level,kind}` (derived, see above). Pagination is by whole
  workout, so a workout's matching sets are never split across pages.
  `total_sets` and `total_workouts` cover the entire filtered result, not just
  the page.
- `GET /api/fitness/calendar` accepts no query parameters and returns
  `{version,days:[{date,volume_points}]}` for every `America/New_York` date
  with at least one set, in ascending date order. `volume_points` follows the
  site set-log score exactly: warm-up = 0, failure = 6, RPE 10/9/8 = 5/4/3,
  and any other or missing effort = 2.
- `GET /api/fitness/workouts/latest` accepts no query parameters and returns
  `{version,workout,newer_workout_path,older_workout_path}` for the newest
  workout by source instant. `workout` has the same shape as a
  `sets`-response workout and is `null` for an empty archive; both neighbor
  paths are then `null` too.
- `GET /api/fitness/workouts/by-path/{path}` accepts one canonical public path
  segment, such as `2026-07-11T20-33-27-04-00`, and returns the same detail
  envelope or 404. The timestamp and offset are the `America/New_York`
  projection, so the offset distinguishes the repeated hour when DST ends.
- `GET /api/fitness/facets` accepts no query parameters and returns
  `{version,summary:{sets,workouts,min_date,max_date},exercises,tags,set_types}`.
  Exercise, tag, and set-type entries are `{value,count}`; `tags` has
  `movement`, `muscle`, and `equipment` arrays. Counts cover the whole archive.
- `GET /api/fitness/ids` accepts no query parameters and returns
  `{ids:string[]}` containing set IDs. The sync command uses these to resume at
  set granularity.

`GET /api/fitness/sets` accepts only these query parameters:

- Text/facets: `q`; exact `exercise`; repeated `movement`, `muscle`,
  `equipment`, and `set_type`. Repeated choices are ORed within one facet and
  different filters are ANDed. `q` searches workout titles/notes/description,
  exercise names, raw exercise names, and exercise notes.
- Dates: inclusive `from`/`to` (`YYYY-MM-DD`); `weekday` = `sun` through `sat`;
  `time_of_day` = `morning` (05:00-11:59), `afternoon` (12:00-16:59),
  `evening` (17:00-20:59), or `night` (21:00-04:59). All date, weekday, and
  time-of-day filtering uses `America/New_York`, including DST transitions.
- Numbers: `min_load`/`max_load` are pounds converted exactly to
  stored thousandths; `min_reps`/`max_reps` are integers; `max_effort` is a
  decimal converted exactly to stored hundredths.
- Flags: `has_record`, `has_superset`, `has_notes`, and `incomplete` accept
  `true` or `false`; `duration` is `normal` or `suspicious`. A set is
  incomplete when reps, distance, and set duration are all absent.
- Pagination: positive `page`; `per_page` is exactly `10`, `20`, or `40`
  (default `20`).

Unknown, duplicated singular, malformed, out-of-range, or contradictory
filters return 400. Repeated facets are capped at eight values each. Search
patterns keep the original 50-byte escaped-LIKE limit as contract.

The write path is `POST /api/fitness/import`, protected by the
`FITNESS_SYNC_TOKEN` secret. The body is capped at 1,000,000 bytes, 50 sets,
50 workouts, 75 exercises, and 300 tags. Its exact shape is:

```text
{
  workouts: [{
    id, title, raw_title, started_at_utc, duration_seconds,
    duration_suspicious, notes, description, source: "workout-data-csv"
  }],
  exercises: [{
    name, tags: [{kind: "movement"|"muscle"|"equipment", value}]
  }],
  sets: [{
    id, workout_id, ordinal, exercise_name, raw_exercise_name, exercise_note,
    superset_id, weight_milli, weight_unit: "lbs", reps, effort_hundredths, distance_milli,
    set_time_seconds, set_type
  }]
}
```

A payload that includes a `records` key is rejected — records are derived,
never imported. Nullable fields must be explicit JSON `null`. IDs,
cross-references, UTC source dates, ordinals, scaled integers, enum values,
and every string/array bound are validated before any write. The server
derives the Eastern fields; callers never supply them. The response is
`{received,added,skipped,version}`, where the counts refer to sets. Existing
set IDs are skipped; a conflicting workout ordinal is an error rather than
being silently ignored. Tags are replaced authoritatively for each exercise
included in a chunk. The fitness version increments only when sets or
taxonomy change.
The CSV path is deliberately append-oriented: an already stored set ID is
immutable. Editing, reordering, or deleting old rows in a later export requires
an explicit replacement operation rather than silently rewriting history.
The normal CLI only posts workouts containing a missing set, so taxonomy-only
changes on a fully imported archive likewise need a deliberate re-import/API
call (or will arrive when that exercise is included with a later missing set).

Sync from the machine that has the export:

```sh
just sync-fitness /home/benji/Downloads/WorkoutData.csv --dry-run
just sync-fitness /home/benji/Downloads/WorkoutData.csv
```

The default token file is `~/.config/benjisponge/fitness.token`; installing
the matching `FITNESS_SYNC_TOKEN` secret is covered in
`docs/railway-deploy.md#database-and-secrets`.

## Local development

- Local data lives in the `benjisponge-surrealdb` Docker container (named
  volume `benjisponge-surrealdb-data`, host port `5800`), which `just dev`
  starts. The app bootstraps `src/schema.surql`; startup does not seed data.
  To seed or adopt a fitness schema/import change, start `just dev` in one
  terminal and reset from another. The CSV defaults to
  `/home/benji/Downloads/WorkoutData.csv`; pass another path as the argument:

  ```sh
  just reset-fitness-local
  just reset-fitness-local /path/to/WorkoutData.csv
  ```

  This replaces local fitness data only; it never affects production or local
  Spire fixtures.

## Changing taxonomy or filters

- Taxonomy originates in `exercise_tags()` and `SQUAT_TYPE_EXERCISES`; update
  importer tests with every classification rule.
- Keep taxonomy values aligned with the filter lists, labels, and
  advanced-filter detection in `src/app/interests/lifting/filters.rs`.
- Normal sync is append-only. It does not resend fully imported workouts for a
  taxonomy-only change. No retag command exists: write an explicit
  API/database replacement workflow instead of rerunning normal sync. Reset
  local fitness data when validating.
- Do not substring-match movement names without boundary tests: `throw` contains
  `row`; wrist/Jefferson curls are not biceps curls.

## Production and future logging

- For a new archive, rollout order is: provision the clean database as
  described in `docs/railway-deploy.md`, configure all five connection
  variables and `FITNESS_SYNC_TOKEN`, deploy committed HEAD, exercise a
  data-backed route so the app installs `src/schema.surql`, then run
  `just sync-fitness`.
- The fitness archive intentionally uses reset-and-resync rather than an
  in-place upgrade. To replace production fitness data, open an authenticated
  `/surreal sql` session inside the private Railway database service and run
  this transaction as one line, then resync from the machine with the CSV:

  ```surql
  BEGIN TRANSACTION; DELETE sets RETURN NONE; DELETE exercise_tags RETURN NONE; DELETE exercises RETURN NONE; DELETE workouts RETURN NONE; UPSERT fitness_meta:version SET k = 'version', v = 0 RETURN NONE; COMMIT TRANSACTION;
  ```

  ```sh
  just sync-fitness /home/benji/Downloads/WorkoutData.csv
  ```

  This is destructive for fitness history until the CSV is resynced. The
  database is shared with Slay the Spire data: do not drop the database itself
  and do not touch any Spire tables. The transaction above touches fitness
  records only.
- Never treat local database contents as proof that production has been reset
  or seeded.
- Manual logging is not implemented. Schema reserves `source='manual'`, but the
  import API accepts only `workout-data-csv`; there is no CRUD UI, mutation ID
  policy, or owner auth yet. Decide those explicitly before adding write UI.

## Validation

```sh
just check
just build
node --check src/app/interests/lifting/auto-filter.js
bash -n scripts/dev.sh
bash -n scripts/reset-fitness-local.sh
cd deploy && npx wrangler types --check && npx tsc --noEmit
```

For API changes, also exercise `just reset-fitness-local`, filtered reads, an
idempotent second sync, and `just dev` shutdown cleanup.
