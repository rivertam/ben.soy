# SurrealDB 3.2.3 notes

The server image and Rust SDK are both pinned to `3.2.3`. Treat the integration
as version-sensitive: read the installed crate source or versioned upstream
docs before changing query or schema APIs.

## Project layout

- `src/data.rs` owns the shared connection and schema bootstrap.
- `src/schema.surql` is the complete committed schema.
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

`Data::db()` initializes lazily, uses an eight-second connection timeout,
selects the configured namespace and database, applies `src/schema.surql`,
and verifies health. A failed initialization is not cached, so a later request
can retry after the database becomes available.

## Schema bootstrap

The app executes the committed `DEFINE ... OVERWRITE` statements on its first
data-backed connection. Every response is checked for statement-level errors.
Definitions are idempotent and do not erase records.

There is no migration runner, migration history, or schema CLI workflow.
Production starts from a clean database, then the application installs the
current schema before the sync CLIs load data. Change `src/schema.surql` and
the corresponding Rust models and queries together.

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

The CLI treats separate stdin lines as separate query requests. A transaction
piped to `/surreal sql` must therefore be a single line. Scripts should request
machine-readable output and reject unexpected statement results; the local
fitness reset is the reference.

Snapshot-backed reads intentionally load their scoped dataset and finish
filtering, ordering, and aggregation in Rust. Preserve those API-level
semantics rather than relying on datastore-specific collation or ordering.

See `docs/railway-deploy.md` for the production service and
`docs/fitness.md` for archive reset and import invariants.
