# SurrealDB 3.2.3 notes

The server image and Rust SDK are both pinned to `3.2.3`. Treat the integration
as version-sensitive: read the installed crate source or versioned upstream
docs before changing query or schema APIs.

## Project layout

- `src/data.rs` owns the shared connection and schema bootstrap.
- `src/schema.surql` holds additive table and field definitions.
- `src/data/schema_migrations/` owns forward-only site index definitions;
  `src/data/diary_migrations/` owns the diary's separately fenced schema.
- Domain models live beside their query code under `src/app`.
- `scripts/dev.sh` starts the pinned local server; production uses
  `deploy/surrealdb.Dockerfile`.

The application requires all five connection variables:

```text
SURREALDB_ENDPOINT
SURREALDB_NAMESPACE
SURREALDB_DATABASE
SURREALDB_USERNAME
SURREALDB_PASSWORD
```

`Data::db()` initializes lazily, uses eight-second budgets for connection,
authentication, namespace selection, direct-sync configuration, and health,
and 60-second budgets for each schema/bootstrap stage. It applies
`src/schema.surql`, the site migrations, and the diary migrations. Errors name
the stage that failed. A failed initialization is not cached, so a later
request can retry.

## Schema bootstrap

The app reconciles the committed table and field `DEFINE ... OVERWRITE`
statements on its first data-backed connection. Every response is checked for
statement-level errors. These definitions are idempotent and do not erase
records.

Indexes are deliberately excluded from that reconciliation. On SurrealDB
3.2.3, `DEFINE INDEX OVERWRITE` deletes and rebuilds the physical index even
when its definition is unchanged. `src/data/schema_migrations.rs` applies the
numbered files in `src/data/schema_migrations/` once and records immutable
`site_schema_migrations` ledger rows. The baseline migration uses
`IF NOT EXISTS`, adopting existing production indexes without rebuilding them.
Add a new migration and bump `CURRENT_SCHEMA_EPOCH` for every later index
change; use `OVERWRITE` only when that one-time rebuild is intentional. Site
epochs are contiguous and additive: an older binary accepts newer ledger rows
so an epoch-1 rollback can continue using its old raw analytics loader. Diary
epochs remain strictly version-fenced and do not share this rule.

Analytics epoch 2 owns `analytics_visitor_days`, its rebuild function/event,
and the leased backfill cursor. The background task is opt-in
(`ANALYTICS_FACTS_BACKFILL=1`) and starts only after bootstrap returns; while it
is scanning or reconciling, request-path rebuilds stay off so live writes only
dirty keys. Facts do not become the dashboard source until all four supported
windows have exact legacy parity. See `docs/analytics.md`.

The diary is the exception to pure reconciliation because offline clients need
an exact activation boundary. `src/data/diary_migrations.rs` applies its
numbered SurrealQL files in order and records immutable ledger rows in the same
transaction as each migration. Add a migration and bump
`CURRENT_SCHEMA_EPOCH` together; never edit an applied migration. A binary
behind the ledger fails initialization, so rollback across an epoch needs an
explicit reverse migration. Diary adapters also recheck the ledger through
`Data::diary_db()` on cached connections. Epoch deploys must drain the old web
process before the new one applies a migration: the check fences later uses,
not an already-running request. Other domains still change table/field
definitions in `src/schema.surql` and their Rust models/queries together;
index changes go through the site migration ledger.

## Query rules

Use the shared client and check both transport and statement results:

```rust
let mut response = db
    .query(
        "SELECT *, game ?? 'sts2' AS game,
                   run_id ?? record::id(id) AS id
         FROM spire_runs",
    )
    .await?
    .check()?;
let rows: Vec<SpireRun> = response.take(0)?;
```

An awaited query can succeed at the protocol level while an individual
statement failed; omitting `.check()` hides that failure.

- Bind values rather than interpolating user input.
- Use `type::record(...)` to construct record references and
  `record::id(id)` when returning their string keys to the public API.
- Keep externally visible IDs as strings in Rust.
- Put related mutations in one explicit `BEGIN TRANSACTION; ... COMMIT
  TRANSACTION;` query so their invariants change atomically.
- Preserve the existing retry and idempotency behavior around write
  conflicts and sync requests.
- Keep scaled integer storage where the schema and API contracts use it;
  do not introduce floating-point drift.

Rules that fail *silently* rather than loudly. Each one produced a working
write that the Rust side then misread:

- **`CREATE`/`CREATE ONLY` return `id` as a record id, not a string.**
  Deserializing the created record into a model whose `id` is `String` fails
  even though the row was written — so a successful create looks like a lost
  race. Return the key explicitly: `RETURN VALUE record::id(id)`.
- **`SELECT *` omits `option` fields holding `NONE`.** It does not return them
  as null, so a model with `Option<T>` fields can fail to deserialize on
  exactly the rows where the value is absent. Project every field explicitly
  when any of them is optional; an explicit projection does yield null.
- **`ORDER BY` requires the field to be in the projection.** Ordering by a
  column the `SELECT` does not return is a parse error ("Missing order idiom"),
  not a silently ignored sort.
- **Two writes to one record in the same instant abort one of them.** The loser
  gets "There was a problem with the key-value store: Transaction conflict:
  Resource busy. This transaction can be retried". Measured 1 failure in 120 for
  two racers and 171 in 800 for eight. The SDK flattens it to a message with no
  typed variant, so recognizing it means matching that string — upstream's own
  tests do the same. `analytics/db.rs::retrying_conflicts` is the reference
  handling: every analytics beacon from one visitor upserts the same
  `analytics_sessions` row, so this is that module's ordinary case rather than a
  rare one. Any new write on a record several requests share needs the same
  wrapper, and it must be a *retry*, not a swallow — the losing write did not
  happen.
- **`DELETE ... WHERE field IN [..]` can match nothing where `SELECT` matches.**
  Observed on `exercise_tags`, whose UNIQUE index is compound
  (`exercise_name, kind, value`) and whose predicate covered only the leading
  field: `SELECT count()` returned 1, the `DELETE` reported success and removed
  nothing. The `=` form deleted the same row correctly. `exercises`, with a
  single-field index, deleted fine either way. Write deletes as one `=` per
  value and verify the row count afterwards — a delete that quietly no-ops is
  indistinguishable from success in the statement results.

The CLI treats separate stdin lines as separate query requests. A transaction
piped to `/surreal sql` must therefore be a single line — and so must a `LET`
and the query that uses it, or the parameter is gone by the time the query
runs and the filter silently matches nothing. Scripts should request
machine-readable output and reject unexpected statement results; the local
fitness reset is the reference.

Snapshot-backed reads intentionally load their scoped dataset and finish
filtering, ordering, and aggregation in Rust. Preserve those API-level
semantics rather than relying on datastore-specific collation or ordering.

See `docs/railway-deploy.md` for the production service and
`docs/fitness.md` for archive reset and import invariants.
