# Fitness archive

Read this before changing `/lifting`, workout import, fitness API/schema, tags,
or local fitness startup. The exact API/filter/import contracts are in this
file under "API contract".

## Data flow

- Page, filter/query handling, API reader, and HTML rendering:
  `src/app/interests/lifting/`; styles are Tailwind utilities inline in those
  views (no section stylesheet). `/lifting` is the
  landing view (daily volume heatmap plus the newest lift; the heatmap day
  preview uses the Topcoat runtime + a shard),
  `/lifting/log` is the filterable full archive, and
  `/lifting/YYYY-MM-DDTHH-MM-SS-04-00` (or `-05-00`) is a complete permanent
  workout page. Its timestamp is the `America/New_York` projection of the
  source instant; the explicit Eastern offset keeps same-date workouts and the
  repeated fall DST hour distinct without exposing importer IDs.
  `/lifting/log` filter chrome is an always-visible search field, a compact tag
  bar (removable active filters), and a two-step “add filter” picker
  (category → value); on wide viewports it sits in the right gutter, otherwise
  inline above the archive. `auto-filter.js` only swaps the no-JS `<details>`
  fallback for the popover “+ filter” button — links and mini GET forms remain
  the navigation path. Page size lives with the pager at the top of the set
  log, not in the filter picker.
- `/lifting/log` also renders the volume heatmap, restricted to the sets the
  active filters admit. That calendar comes from
  `Snapshot::calendar_filtered` in-process (the same per-set predicate as the
  set log); the public `/api/fitness/calendar` endpoint deliberately still
  accepts no query parameters. Heatmap day cells carry the active filters
  (minus `from`/`to`/`page`) into their day-log links. A logged day opens a
  shared popover whose body is a Topcoat shard (`day_preview_shard`): the
  calendar SSR stays volume-only, and the day's titles / exercise names /
  compact muscle maps load on hover or click. Shard args are untrusted and
  validated (`YYYY-MM-DD` plus a filter-shaped `link_query`). Hover/pin
  chrome is `heatmap-preview.js`; click-to-open still works via native
  `popovertarget`. Previews remain full-day even when filters only lit the
  cell, and never widen the calendar JSON.
- Muscle involvement is driven by stored weighted connections, not tags:
  `exercise_muscles` rows carry `(exercise_name, granular muscle,
  ratio_hundredths 1..=100)`; absence of a row means no credit. The
  granular vocabulary (28 muscles in 9 display groups — delt heads, traps
  thirds, chest bands, …) lives in `muscle_taxonomy.rs`; the schema ASSERT
  lists mirror it and a test keeps them aligned. Workout pages derive the
  front/back SVG map at render time from those ratios alone: at or above
  `muscles::PRIMARY_THRESHOLD` (75) a muscle shades primary, any smaller
  stored ratio secondary. No rank is stored — like records, the split is
  derived, and weights reach pages through `Snapshot::exercise_weight_map`,
  never JSON.
- `/lifting/exercise/{urlencoded-name}` shows one exercise's ratios, tags,
  and history; the signed-in `ADMIN_EMAIL` sees the same page with editable
  0–100 inputs. `POST /lifting/exercise/{name}` repeats the admin check,
  requires same-origin evidence, bounds and strictly decodes the form
  (exactly one field per canonical muscle), rejects all-zero saves (they
  would re-open the exercise to reseeding), replaces the exercise's rows
  with `source='admin'` in one transaction, bumps the fitness version, and
  rebuilds the snapshot — every page reflects an edit immediately.
- `/lifting` derives its muscle-load and next-focus panel from those same
  weights; it is page-only, not a stored record or public API field. Credit
  accumulates in exact integer centi-points (`set volume points ×
  ratio_hundredths`, `scoring::muscle_credit_centi`); display divides by
  100 once, rounding half-away-from-zero. The past seven Eastern days are
  compared per granular muscle with its own weekly pace over the preceding
  eight weeks, and the panel renders group header rows with one bar per
  granular muscle. Load rows link to the log through the granular→coarse
  tag mapping (`muscle_taxonomy::coarse_tag_for`) because the `muscle`
  facet deliberately keeps the original 13-value tag vocabulary.
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
- Schema: `src/schema.surql` — eight fitness tables: `workouts`,
  `exercises`, `exercise_tags`, `sets`, `fitness_meta`, `muscles` (the
  granular vocabulary as data), `exercise_muscles` (weighted
  connections, deterministic record key = sha-256 of `exercise\nmuscle`),
  and `fitness_interruptions` (annotate-only Eastern date ranges with a
  free-text note and curated heatmap emoji — illness, travel, and the like;
  `to_date` is optional for an open/ongoing interruption; never feed volume
  points, records, calendar JSON, or training-focus pace).
- CSV parsing, stable IDs, taxonomy, chunking:
  `src/app/interests/lifting/fitness_sync.rs`; taxonomy shared by that binary
  and browser uploads lives in `src/app/interests/lifting/taxonomy.rs`.
- For the signed-in `ADMIN_EMAIL` only, `/lifting` shows an "upload lift"
  dialog for pasting one Lyfta share. `POST /lifting/upload` independently
  repeats that exact admin check, requires positive same-origin browser
  evidence, bounds and strictly decodes the form, parses the text in
  `archive/manual.rs`, and uses the create-only write in `archive/db.rs`.
  A successful workout has `source='manual'`, appears immediately throughout
  `/lifting`, and joins the `/` timeline and `/feed.xml`; CSV history
  never joins either feed. The
  progressive clipboard action reads the copied Lyfta share, publishes it
  through that same POST, then replaces the clipboard with the canonical text
  generated by `lifting/share.rs` before opening the workout. The normal
  textarea and form remain the no-JavaScript, denied-permission, and
  unsupported-browser fallback.
- Annotate-only interruptions: the signed-in `ADMIN_EMAIL` sees a “log
  interruption” dialog next to “upload lift” on `/lifting`, plus edit /
  delete on open rows there and on closed rows in the `/lifting/log`
  timeline. Writes are `POST /lifting/interruptions`,
  `POST /lifting/interruptions/{id}`, and
  `POST /lifting/interruptions/{id}/delete` — each repeats the admin check
  and requires same-origin evidence. Inclusive Eastern `from` (`YYYY-MM-DD`),
  optional `to` (blank = open), a free-text `note` (1..=200), and a curated
  heatmap `emoji`. Opaque 32-hex ids keep identity across edits. Overlaps
  and multiple open rows are allowed. Open interruptions (no `to`) appear
  only in the `/lifting` notes section — that section is omitted when none
  are open. Closed interruptions inject into the `/lifting/log` set list by
  `to_date` (after same-day workouts; omitted from pager counts). The heatmap
  marks covered days with that emoji (open rows through today Eastern;
  closed through `to`; newest wins on overlap) and surfaces emoji + note in
  the day preview. Interruptions are page-only (not part of
  `/api/fitness/*`). Bumps the fitness version and rebuilds the snapshot so
  every page reflects the change immediately.
- `just dev [port]` delegates to `scripts/dev.sh`: it starts the local
  SurrealDB container, then runs Topcoat with local-only sync tokens. The app
  applies the committed schema on its first data-backed connection. It never
  imports data. It also starts the Podrick Discord bot when `.env.dev`
  configures one (`docs/podrick.md`), which reads fitness tables but never
  writes them; `just dev --no-podrick` skips it.
- `just reset-fitness-local [csv]` runs while `just dev` is active. It
  truncates only the local fitness tables (including
  `fitness_interruptions`), resets the fitness version, and
  imports the CSV; local Spire tables in the shared database remain untouched.
  This intentionally deletes locally pasted manual workouts and interruption
  notes too.

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
- Lyfta shares label their timestamp in the user's local wall clock. The
  browser parser interprets it as `America/New_York`, rejects DST gaps and
  folds (the text has no offset with which to disambiguate them), converts it
  to the UTC identity anchor, and keeps Lyfta's minute precision by using
  seconds `00`. The displayed weekday and declared exercise/set counts must
  agree with the parsed body. The reported aggregate volume is informational:
  do not recompute or validate it from the set rows.
- Browser uploads accept pounds only, preserve whole-workout set order, map
  `(Warm Up)`/`(Failure)` and supported set annotations to the existing set
  types, and convert `rir` to stored RPE hundredths. The same taxonomy
  classifier as the CSV importer handles exercises. Lyfta's numbered exercise
  headings (`3. Pec Fly`) shed their position prefix before storage — the
  prefix is share-sheet furniture, and keeping it would sever the exercise
  from its history and its derived records; a numbered heading must match its
  1-based position in the workout.
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
The delete path is `DELETE /api/fitness/workouts/by-path/{path}` — the same
resource the GET above serves, addressed by the same canonical path segment.
It is the explicit replacement operation the append-only rules below reserve
for corrections: delete, then repaste or resync.

- Two authorizations, either sufficient: `Authorization: Bearer
  $FITNESS_SYNC_TOKEN` (scripts, no browser evidence needed), or the
  signed-in `ADMIN_EMAIL` viewer cookie *plus* `is_same_origin` evidence.
  Hidden-page grants never authorize it. Anyone else gets 401; a signed-in
  non-admin gets 404, like `/lifting/upload`.
- 200 returns `{path,workout_id,source,sets_deleted,version}`. `source` is
  the deleted workout's, because deleting a `workout-data-csv` workout is
  undone by the next `just sync-fitness` — sync resends any workout holding
  a missing set. Remove it from the export first, or delete only `manual`
  workouts.
- 404 for a malformed path, an unparseable one, or a path with no workout —
  the same answer the GET gives. Deliberately not idempotent-by-204: a typo'd
  path must not look like a successful delete.
- 400 for any query string. The response carries no
  `Access-Control-Allow-Origin`; the GET on the same URL still does.
- Removes exactly the `workouts` row, its `sets`, and bumps the version.
  `exercises`, `exercise_tags`, and `exercise_muscles` rows survive even
  when the deleted workout held an exercise's last set: the snapshot never
  loads the `exercises` table and every public count joins through sets, so
  orphans are invisible rather than merely harmless, and hand-corrected
  taxonomy and weights survive a delete-and-repaste. Records need no
  cleanup — they are derived at snapshot build, so the remaining history
  re-derives its own podium.
- The snapshot is rebuilt in-process on success, so `/lifting` reflects the
  delete immediately. A rebuild failure is logged, not reported: the commit
  already landed, and a retry would now 404.

```sh
just delete-lift 2026-07-27T13-42-00-04-00           # prompts, confirms by path
just delete-lift <workout URL> --yes                 # accepts a pasted /lifting/ URL
just delete-lift <path> --api http://127.0.0.1:3000  # local
```

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

The browser write is separate from that API and is authorized only by
`content::access::ADMIN_EMAIL`; hidden-page grants never authorize it. Pasted writes are create-only: an exact repeat redirects to the existing
permanent workout, while the same deterministic timestamp ID with different
workout content returns 409. Existing exercise taxonomy is preserved;
taxonomy is inserted only for a new exercise. The JSON sync endpoint
continues to accept only `source='workout-data-csv'`.

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
  Spire fixtures. It also removes manual workouts from the local archive; they
  cannot be reconstructed from the CSV.

## Muscle weights

- Seeding is insert-only at exercise granularity:
  `db::reconcile_muscle_weights` runs at the top of every snapshot load,
  upserts the `muscles` vocabulary rows, and gives weight rows only to
  exercises that have none — the researched `muscle_seed::SEED_WEIGHTS`
  table first (source `seed`), else ratios derived from the exercise's
  taxonomy tags at the old primary=100/secondary=50 split expanded to
  granular constituents (source `derived`). Any existing row — including
  `admin` — suppresses seeding for that whole exercise, so hand-tuned
  ratios are authoritative forever, the same invariant hand-corrected
  taxonomy has. In steady state reconcile reads and writes nothing; it
  never bumps the version (the same call builds the snapshot).
- Pure-cardio exercises (Running, Rowing, Stair Stepper) are deliberately
  absent from the seed table and stay unseeded: no weights, no muscle
  credit. The admin form rejects an all-zero save for the same reason — it
  would delete every row and re-open the exercise to reseeding.
- New exercises arriving via CSV sync or Lyfta paste get weights on the
  next snapshot load (the post-write `rebuild()` triggers it); neither
  wire contract changed.
- `exercise_muscles` carries a compound UNIQUE `(exercise_name, muscle)`
  index: deletes must use one `=` predicate per pair, never `IN [..]`
  (docs/surrealdb-notes.md).
- The muscle facet filter and `exercise_tags` are unchanged — tags answer
  "does this exercise involve X" at the coarse 13-value scale, weights
  answer "how much" at the granular scale. An admin weight for a muscle
  whose coarse tag the exercise lacks will not surface in the filter;
  accepted drift, audited by `.claude/skills/audit-muscle-weights`.

## Changing taxonomy or filters

- Taxonomy originates in `exercise_tags()` and `SQUAT_TYPE_EXERCISES`; update
  importer tests with every classification rule.
- Keep taxonomy values aligned with the filter lists, labels, and add-filter
  categories in `src/app/interests/lifting/filters.rs` /
  `src/app/interests/lifting/filter_ui.rs`.
- Normal sync is append-only. It does not resend fully imported workouts for a
  taxonomy-only change. No retag command exists: write an explicit
  API/database replacement workflow instead of rerunning normal sync. Reset
  local fitness data when validating.
- Do not substring-match movement names without boundary tests: `throw` contains
  `row`; wrist/Jefferson curls are not biceps curls.

## Production and browser logging

- For a new archive, rollout order is: provision the clean database as
  described in `docs/railway-deploy.md`, configure all five connection
  variables and `FITNESS_SYNC_TOKEN`, deploy committed HEAD, exercise a
  data-backed route so the app installs `src/schema.surql`, then run
  `just sync-fitness`.
- The CSV corpus uses reset-and-resync rather than an in-place upgrade. Manual
  workouts are not present in that CSV and must survive replacement. Run this
  transaction as one line against the production database (see "Running SQL
  against production" below), then resync from the machine with the CSV:

  ```surql
  BEGIN TRANSACTION; DELETE sets WHERE workout_id IN (SELECT VALUE record::id(id) FROM workouts WHERE source = 'workout-data-csv') RETURN NONE; DELETE workouts WHERE source = 'workout-data-csv' RETURN NONE; UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1 RETURN NONE; COMMIT TRANSACTION;
  ```

  ```sh
  just sync-fitness /home/benji/Downloads/WorkoutData.csv
  ```

  This is destructive for the CSV-backed history until it is resynced, but
  leaves manual workouts, their sets, and shared exercise taxonomy intact.
  Orphaned exercise rows are harmless because every public count joins through
  sets. The database is shared with Slay the Spire data: do not drop the
  database itself and do not touch any Spire tables.
- Never treat local database contents as proof that production has been reset
  or seeded.
- Lyfta shares expose only minute precision. A later CSV export may carry
  seconds and therefore derive a different ID for the same real workout.
  Until there is an explicit reconciliation key, do not ingest the same
  post-baseline session through both browser paste and CSV sync.
- There is no edit UI for workouts, and no delete *button* on a lift page
  beyond the deliberate admin control — deleting a workout is a deliberate
  command (`just delete-lift`, above), not something a mis-click can do.
  Workout writes stay create-only: correcting a workout means deleting it and
  publishing it again, never editing set history or derived records in
  place. A delete leaves any Podrick announcement row behind, so a
  delete-and-repaste does not re-announce to Discord (`docs/podrick.md`).
  Interruptions are the other write exception: admin form POSTs may create,
  edit, and delete annotate-only date-range notes without touching sets.

## Running SQL against production

Reach for this only for what no endpoint covers — deleting one workout is
`just delete-lift`, not a hand-written transaction.

There is no SQL prompt inside the `surrealdb` service: that image is
distroless (just the `/surreal` binary, no shell), so `railway ssh` into it
can never give you one. Go through the **web** service instead. It is
debian-slim, sits on the same private network, and already holds the five
`SURREALDB_*` variables. It has no `curl`, so speak SurrealDB's HTTP `/sql`
endpoint over bash's `/dev/tcp`:

```sh
railway link --project <project> --environment <env> --service <web>
railway ssh --service <web> bash -c '
  SQL="<one-line transaction>"
  endpoint=${SURREALDB_ENDPOINT#*://}; host=${endpoint%%:*}; port=${endpoint##*:}
  auth=$(printf "%s:%s" "$SURREALDB_USERNAME" "$SURREALDB_PASSWORD" | base64 -w0)
  exec 3<>"/dev/tcp/$host/$port"
  printf "POST /sql HTTP/1.1\r\nHost: %s:%s\r\nAuthorization: Basic %s\r\nAccept: application/json\r\nsurreal-ns: %s\r\nsurreal-db: %s\r\nContent-Type: text/plain\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s" \
    "$host" "$port" "$auth" "$SURREALDB_NAMESPACE" "$SURREALDB_DATABASE" "${#SQL}" "$SQL" >&3
  cat <&3'
```

`railway ssh` needs a registered SSH key on the Railway account. Read the
response: a statement can report `"status":"OK"` and still have changed
nothing (see the `DELETE ... IN [..]` rule in `docs/surrealdb-notes.md`), so
verify row counts afterwards rather than trusting the status.

## Validation

```sh
just check
just build
node --check src/app/interests/lifting/auto-filter.js
bash -n scripts/dev.sh
bash -n scripts/reset-fitness-local.sh
bash -n scripts/delete-lift.sh
cd deploy && npx wrangler types --check && npx tsc --noEmit
```

For API changes, also exercise `just reset-fitness-local`, filtered reads, an
idempotent second sync, and `just dev` shutdown cleanup.
