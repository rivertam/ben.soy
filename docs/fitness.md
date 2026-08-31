# Fitness archive

Read this before changing `/fitness`, workout/run/step import, fitness API/schema,
tags, or local fitness startup. Health Connect setup and the step wire contract
are in [steps-sync.md](steps-sync.md). The exact lifting
API/filter/import contracts are in this file under "API contract".

The private, OAuth-protected agent interface is documented separately in
[fitness-mcp.md](fitness-mcp.md). It shares these data invariants but does not
reuse the local sync token or expose unrestricted SurrealQL.

## Data flow

- Page, filter/query handling, API reader, and HTML rendering compose the
  independently stored lifting, running, and daily-step models under one public surface.
  `/fitness` is the landing view (combined training heatmap, recent step chart,
  and the newest lift and run), `/fitness/log` is the filterable, UTC-interleaved activity
  archive, `/fitness/lift/YYYY-MM-DDTHH-MM-SS-04-00` (or `-05-00`) is a
  complete workout page, and `/fitness/run/{Eastern-start}/{sha256-id}` is a
  run summary. Legacy `/lifting` and `/running` reads permanently redirect to
  their canonical Fitness equivalents. A lift timestamp is the
  `America/New_York` projection of the
  source instant; the explicit Eastern offset keeps same-date workouts and the
  repeated fall DST hour distinct without exposing importer IDs.
  Anonymous `/fitness/log` HTML is browser-cold and edge-cacheable for 60
  seconds (`max-age=0`, `s-maxage=60`); viewer-cookie requests are forced to
  `private, no-store` by the site response layer and bypassed at Cloudflare.
  The standalone `/fitness` page, detail pages, and fitness APIs remain
  `no-store`.
  `/fitness/log` filter chrome is an always-visible search field, a compact tag
  bar (removable active filters), and a two-step “add filter” picker
  (category → value); on wide viewports it sits in the right gutter, otherwise
  inline above the archive. `auto-filter.js` only swaps the no-JS `<details>`
  fallback for the popover “+ filter” button — links and mini GET forms remain
  the navigation path. Page size lives with the pager at the top of the
  activity log, not in the filter picker.
- `/fitness` and `/fitness/log` render one layered training calendar. Oxide
  fill intensity remains strictly lifting volume, a patina edge marker means
  one or more runs occurred, and the existing emoji annotates an interruption;
  all three can coexist. Run-only days are interactive but add zero volume
  points. The filtered lifting calendar comes from
  `Snapshot::calendar_filtered` in-process (the same per-set predicate as the
  set log), while eligible runs are composed in the page layer; the public
  `/api/fitness/calendar` endpoint deliberately remains lifting-only and still
  accepts no query parameters. `from`, `to`, `weekday`, and `time_of_day`
  apply to both activity types. Any set-specific predicate excludes runs,
  because a run cannot satisfy an exercise/load/set condition. Heatmap day
  cells carry the active filters
  (minus `from`/`to`/`page`) into their day-log links. A logged day opens a
  shared popover whose body is a Topcoat shard (`day_preview_shard`): the
  calendar SSR stays compact, and that day's lifts, runs, exercise names, and
  compact muscle maps load on hover or click in exact UTC order. Shard args are untrusted and
  validated (`YYYY-MM-DD` plus a filter-shaped `link_query`). Hover/pin
  chrome is `heatmap-preview.js`; click-to-open still works via native
  `popovertarget`. Previews remain full-day even when filters only lit the
  cell, and never widen the lifting calendar JSON. The activity log paginates
  lifts and runs together; interruptions consume no slots and render after all
  primary activities on their ending date.
- `/fitness` also reads the 35 newest `daily_steps` rows directly and renders a
  14-day step chart. Steps remain deliberately outside the training heatmap,
  activity log, lifting snapshot/version, scoring, records, muscle load, feeds,
  and Podrick. Their Android ingestion and setup are in
  [steps-sync.md](steps-sync.md).
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
- `/fitness/exercise/{urlencoded-name}` shows one exercise's ratios, tags,
  and history; the signed-in `ADMIN_EMAIL` sees the same page with editable
  0–100 inputs. `POST /fitness/exercise/{name}` repeats the admin check,
  requires same-origin evidence, bounds and strictly decodes the form
  (exactly one field per canonical muscle), rejects all-zero saves (they
  would re-open the exercise to reseeding), replaces the exercise's rows
  with `source='admin'` in one transaction, bumps the fitness version, and
  rebuilds the snapshot — every page reflects an edit immediately.
- `/fitness` derives its lifting-only muscle-load and next-focus panel from those same
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
- Schema: `src/schema.surql` — ten fitness tables: `workouts`,
  `exercises`, `exercise_tags`, `sets`, `fitness_meta`, `muscles` (the
  granular vocabulary as data), `exercise_muscles` (weighted
  connections, deterministic record key = sha-256 of `exercise\nmuscle`),
  and `fitness_interruptions` (annotate-only Eastern date ranges with a
  free-text note and curated heatmap emoji — illness, travel, and the like;
  `to_date` is optional for an open/ongoing interruption; never feed volume
  points, records, calendar JSON, or training-focus pace), and
  `running_activities` (small, route-free summaries from Garmin or the manual
  distance-and-time form; kept entirely outside lifting's set snapshot and
  scoring), and `daily_steps` (date-keyed, intentionally mutable Health Connect
  aggregates with a source-export watermark).
- CSV parsing, stable IDs, taxonomy, chunking:
  `src/app/interests/lifting/fitness_sync.rs`; taxonomy shared by that binary
  and browser uploads lives in `src/app/interests/lifting/taxonomy.rs`.
- For the signed-in `ADMIN_EMAIL` only, the `/fitness` and main log headers
  share the same “log” launcher with Lift, Run, and Interruption choices. Each
  choice opens its own native dialog;
  the launcher options remain fragment links and the forms render in flow
  under `<noscript>`, so the ordinary form POSTs remain usable without the
  dialog enhancement. The Lift dialog accepts one pasted Lyfta share.
  `POST /fitness/lift/import` independently
  repeats that exact admin check, requires positive same-origin browser
  evidence, bounds and strictly decodes the form, parses the text in
  `archive/manual.rs`, and uses the create-only write in `archive/db.rs`.
  A successful workout has `source='manual'`, appears immediately throughout
  `/fitness`, and joins the `/` timeline and `/feed.xml`; CSV history
  never joins either feed. The
  progressive clipboard action reads the copied Lyfta share, publishes it
  through that same POST, then replaces the clipboard with the canonical text
  generated by `lifting/share.rs` before opening the workout. The normal
  textarea and form remain the no-JavaScript, denied-permission, and
  unsupported-browser fallback.
- Running is a sibling fitness storage model, not a synthetic lifting set.
  `src/app/interests/running/` owns both the Garmin adapter and manual run
  entry, their route-free summaries, and the shared create-only write, while
  the page layer composes runs with lifts and interruptions. The identity
  suffix keeps simultaneous activities independently reachable. Runs never
  enter `FitnessStore`, lifting volume points, records, muscle load, set
  facets, or Podrick. They do join the combined heatmap/log, `/` timeline, and
  `/feed.xml` as their own item kind.
- The Run choice keeps the deliberately small manual path primary: distance in
  miles plus elapsed time as total minutes and seconds. Garmin Connect import
  is folded into the same dialog as a secondary disclosure rather than a
  separate Fitness-header control. `POST /fitness/run/manual`
  strictly parses and bounds those integer form values, converts them to the
  same integer millimeters and milliseconds used by imported runs, and stamps
  the activity's start from server time at the first successful write. It
  stores `source='manual'`, title `Run`, activity type `running`, and no moving
  duration or ascent. The server-rendered form carries a fresh random 64-hex
  submission token; the record id is sha-256(`manual\n{token}`). Replaying the
  same form with the same metrics is idempotent even after the clock advances:
  the first stored timestamp wins and the replay redirects to that existing
  run. Reusing the token with different metrics returns 409 and never
  overwrites history. The POST independently requires exact `ADMIN_EMAIL`,
  positive same-origin evidence, the bounded URL-encoded form, and returns
  `no-store` / `no-referrer` responses on every path.
- The Android PWA share path is review-before-write. Install `/fitness` once,
  then Garmin Connect's generic URL share sends `title`, `text`, and `url` in
  a bounded `POST /fitness/share` form; Android may place the URL in any of
  those three fields, so the parser scans all of them. The write-free ingress
  accepts only one exact HTTPS `connect.garmin.com` activity URL, extracts its
  digits-only activity id, then 303-redirects to the canonical GET review URL.
  Raw share text therefore never enters browser history or ingress access
  logs. The adapter reconstructs the fixed
  `/embed/activity/{id}` fetch itself. It never fetches a caller-selected
  host. A signed-out launch carries only that numeric id through OAuth's
  bounded `next`; a signed-in launch immediately canonicalizes the address to
  that same id before fetching. A non-admin gets a real 404.
- Garmin's public embed is an undocumented convenience boundary, so imports
  require the activity privacy to be **Everyone** during review and commit.
  The adapter uses no Garmin credentials, disables redirects, bounds time and
  response bytes, and inertly decodes only the title/type/start/duration/
  moving-duration/distance/ascent summary and a canonical
  `https://connect.garmin.com/app/activity/{id}` source link. It deliberately
  does not retain the raw response, map/polyline, coordinates, profile, device,
  heart-rate/cadence samples, calories, or splits. The review page performs no
  write; `POST /fitness/run/import` repeats exact-admin and same-origin checks,
  re-fetches by numeric id, rejects any summary whose digest differs from the
  reviewed normalization, then uses deterministic
  sha-256(`garmin-connect\n{id}`) identity with `CREATE ONLY`. An exact replay
  redirects to the existing run; conflicting content is never overwritten.
  Transient SurrealDB unique-index conflicts get the documented bounded retry.
  The activity can be made private in Garmin again after a successful import.
  This scrape path remains separate from manual entry: it supplies Garmin's
  absolute source instant and optional moving-time/ascent fields, while a
  manual run needs neither a Garmin URL nor Garmin visibility.
- The same share target also recognizes Lyfta when Android supplies the
  complete plain-text workout (a link alone is insufficient). It reconstructs
  title/text/URL field splits, passes candidates through the existing strict
  Lyfta parser, and renders the full text in an escaped review textarea.
  Nothing writes at native-share ingress. A deliberate second form POST to
  `/fitness/lift/import` repeats admin, same-origin, bounds, parsing, and
  create-only checks. Signed-out Lyfta text is never staged through OAuth; the
  viewer is asked to sign in and share again. Unknown, link-only, mixed
  Garmin+Lyfta, and multi-activity shares fail without writing.
- `/fitness.webmanifest`, `/fitness/sw.js`, and the tiny registration module
  form a separate PWA scoped to `/fitness`. Its worker has no fetch handler,
  cache, or offline data. Never add the share target to the diary manifest:
  Web Share Target actions must be inside their manifest scope, while the
  diary intentionally owns only `/diary`. Platforms without incoming Web
  Share Target support use the manual fields or nested Garmin URL form in the
  Run dialog, or the Lyfta text form in the Lift dialog on `/fitness`.
- Annotate-only interruptions: the Interruption choice in the owner-only log
  launcher opens its create dialog; edit / delete controls remain on open rows
  on `/fitness` and closed rows in the `/fitness/log`
  timeline. Writes are `POST /fitness/interruptions`,
  `POST /fitness/interruptions/{id}`, and
  `POST /fitness/interruptions/{id}/delete` — each repeats the admin check
  and requires same-origin evidence. Inclusive Eastern `from` (`YYYY-MM-DD`),
  optional `to` (blank = open), a free-text `note` (1..=200), and a curated
  heatmap `emoji`. Opaque 32-hex ids keep identity across edits. Overlaps
  and multiple open rows are allowed. Open interruptions (no `to`) appear
  only in the `/fitness` notes section — that section is omitted when none
  are open. Closed interruptions inject into the `/fitness/log` activity list by
  `to_date` (after same-day lifts and runs; omitted from pager counts). The heatmap
  marks covered days with that emoji (open rows through today Eastern;
  closed through `to`; newest wins on overlap) and surfaces emoji + note in
  the day preview. Interruptions are page-only (not part of
  `/api/fitness/*`). Bumps the fitness version and rebuilds the snapshot so
  every page reflects the change immediately.
- `just dev [port]` delegates to `scripts/dev.sh`: it starts the local
  SurrealDB container, then runs Topcoat with local-only sync tokens, including
  `STEPS_SYNC_TOKEN=local-development`. The app
  applies the committed schema on its first data-backed connection. It never
  imports data. It also starts the Podrick Discord bot when `.env.dev`
  configures one (`docs/podrick.md`), which reads fitness tables but never
  writes them; `just dev --no-podrick` skips it.
- `just reset-fitness-local [csv]` runs while `just dev` is active. It
  truncates only the local fitness tables (including `fitness_interruptions`,
  locally imported `running_activities`, and `daily_steps`), resets the
  fitness version, and imports the CSV; local Spire tables in the shared
  database remain untouched.
  This intentionally deletes locally pasted manual workouts, manual and
  Garmin runs, step totals, and interruption notes too.

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
- Set load is signed: a negative `weight_milli` represents assistance. CSV,
  authenticated JSON import, storage, public JSON, pages, shares, and Podrick
  preserve the sign; all other scaled set quantities remain non-negative.
- Strong labels effort `RIR/RPE`. On import, values below 6 are treated as RIR
  and converted with `RPE = 10 - RIR`; values at or above 6 are stored as RPE.
- Lyfta shares label their timestamp in the user's local wall clock. The
  browser parser interprets it as `America/New_York`, rejects DST gaps and
  folds (the text has no offset with which to disambiguate them), converts it
  to the UTC identity anchor, and keeps Lyfta's minute precision by using
  seconds `00`. The displayed weekday and declared exercise/set counts must
  agree with the parsed body. The reported aggregate volume is informational:
  do not recompute or validate it from the set rows.
- Browser uploads accept pounds only, including both Lyfta negative-assistance
  spellings (`- 45lbs` and `-45lbs`), preserve whole-workout set order, map
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
- Records (`/fitness/lift/*` badges) are derived from full set history when the
  in-memory snapshot is rebuilt, never stored or imported. Negative assisted
  sets earn reps records only: assistance alone is not the resistance moved,
  and it is not comparable to older positive-assistance history.
- Run metrics use integer milliseconds and millimeters. Pace is always derived
  at render time; it is never stored. Garmin supplies an absolute source start
  instant; manual entry uses the first write's server timestamp. Both project
  through the same `America/New_York` DST rules as lifts, and that projection
  supplies the public date/path. Manual distance accepts miles and manual
  elapsed time accepts total minutes plus seconds before exact integer scaling.
- Steps are Health Connect calendar-day aggregates, not activity samples and
  not an Eastern projection performed by this site. The record's ISO date and
  source calendar timezone come from Health.md. A later source export may
  authoritatively replace the same date; its millisecond watermark prevents an
  older delayed request from reverting the total.

## API contract

Lifting read endpoints are served from an immutable in-memory snapshot
(`src/app/interests/lifting/archive/snapshot.rs`), rebuilt when the fitness
version changes. The small steps endpoint reads its independent table
directly. Filter semantics deliberately mirror the original Worker SQL
(ASCII-only case folding, byte-order sorts, NULL-excluding comparisons); the
golden fixtures under `tests/fixtures/api` are the contract.

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
  `weight_milli` is a signed integer or `null`, with negatives representing
  assistance;
  each record is `{level,kind}` (derived, see above). Pagination is by whole
  workout, so a workout's matching sets are never split across pages.
  `total_sets` and `total_workouts` cover the entire filtered result, not just
  the page.
- `GET /api/fitness/calendar` accepts no query parameters and returns
  `{version,days:[{date,volume_points}]}` for every `America/New_York` date
  with at least one set, in ascending date order. `volume_points` follows the
  site set-log score exactly: warm-up = 0, failure = 6, RPE 10/9/8 = 5/4/3,
  and any other or missing effort = 2.
- `GET /api/fitness/steps` accepts no query parameters and returns the 35 newest
  daily totals, newest first, as `{days:[{date,steps}]}`. It exposes no import
  timestamp, source timezone, or Health.md failure detail.
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
  stored thousandths. Those filter inputs remain non-negative; `max_load=0`
  includes assisted sets. `min_reps`/`max_reps` are integers; `max_effort` is
  a decimal converted exactly to stored hundredths.
- Flags: `has_record`, `has_superset`, `has_notes`, and `incomplete` accept
  `true` or `false`; `duration` is `normal` or `suspicious`. A set is
  incomplete when reps, distance, and set duration are all absent.
- Pagination: positive `page`; `per_page` is exactly `10`, `20`, or `40`
  (default `20`).

Unknown, duplicated singular, malformed, out-of-range, or contradictory
filters return 400. Repeated facets are capped at eight values each. Search
patterns keep the original 50-byte escaped-LIKE limit as contract.

The mutable step write is `POST /api/fitness/steps`, protected only by the
dedicated `STEPS_SYNC_TOKEN`. It accepts Health.md compatibility API envelope
v1 / daily-record v4, caps the body at 256 KiB and 400 dates, and upserts one
validated `0..=1,000,000` integer total per ISO date. Replays are idempotent;
newer exports may correct a total, while an equal or older source watermark
cannot overwrite it. The receipt is
`{received,accepted,added,updated,unchanged,stale,omitted,failed_dates}`.
Missing/failed dates never erase stored data. This path does not reuse
`FITNESS_SYNC_TOKEN`, bump the lifting version, or rebuild the lifting snapshot.
The exact envelope and phone setup are in [steps-sync.md](steps-sync.md).

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
and every string/array bound are validated before any write. `weight_milli`
is bounded to `-1_000_000_000..=1_000_000_000`; the other set quantities are
non-negative. The server
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
  non-admin gets 404, like `/fitness/lift/import`.
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
- The snapshot is rebuilt in-process on success, so `/fitness` reflects the
  delete immediately. A rebuild failure is logged, not reported: the commit
  already landed, and a retry would now 404.

```sh
just delete-lift 2026-07-27T13-42-00-04-00           # prompts, confirms by path
just delete-lift <workout URL> --yes                 # accepts /fitness/lift/ or legacy /lifting/
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

The browser writes are separate from that API and are authorized only by
`content::access::ADMIN_EMAIL`; hidden-page grants never authorize them. Lift
and manual-run writes also require positive same-origin evidence and remain
create-only. A repeated pasted lift redirects to the existing permanent
workout, while the same deterministic timestamp ID with different workout
content returns 409. A repeated manual-run submission token redirects to the
first stored run when its metrics match and returns 409 when they do not.
Existing exercise taxonomy is preserved; taxonomy is inserted only for a new
exercise. The JSON sync endpoint continues to accept only
`source='workout-data-csv'`.

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
  Spire fixtures. It also removes manual workouts, manual runs, and imported
  Garmin runs from the local archive; none can be reconstructed from the CSV.

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
  variables, `FITNESS_SYNC_TOKEN`, and `STEPS_SYNC_TOKEN`, deploy committed
  HEAD, exercise a data-backed route so the app installs `src/schema.surql`,
  then run `just sync-fitness`.
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
  Daily step upserts are the separate source-authoritative exception described
  in `docs/steps-sync.md`. Interruptions are the other admin write exception:
  form POSTs may create, edit, and delete annotate-only date-range notes
  without touching sets.

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
node --check src/app/interests/running/pwa.js
node --check src/app/running/sw.js
bash -n scripts/dev.sh
bash -n scripts/reset-fitness-local.sh
bash -n scripts/delete-lift.sh
cd deploy && npx wrangler types --check && npx tsc --noEmit
```

For API changes, also exercise `just reset-fitness-local`, filtered reads, an
idempotent second sync, and `just dev` shutdown cleanup.
