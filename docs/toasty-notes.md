# Toasty 0.9.0 crib sheet

Toasty is the Rust ORM behind every row this site stores (spire runs,
fitness archive, analytics). It is pre-1.0 and moves fast — read the
vendored source before reaching for an API, don't guess from other ORMs.

Ground truth:

- Vendored sources: `~/.cargo/registry/src/index.crates.io-*/toasty-0.9.0/`
  (query builder in `src/stmt/`, handles in `src/db/`) and
  `toasty-cli-0.9.0/` (migration commands)
- `toasty-macros-0.9.0/src/lib.rs` — the `Model` derive's doc comment is the
  authoritative attribute reference
- docs.rs: https://docs.rs/toasty/0.9.0 — repo: https://github.com/tokio-rs/toasty
- This repo's usage, best-first: `src/app/interests/spire/db.rs` (queries,
  transaction, idempotent insert), `src/app/analytics/db.rs` (raw SQL,
  `dyn Executor`), `src/app/interests/lifting/archive/db.rs` (the big one)

## Layout

- `src/data.rs` — the `Data` handle, `connect()`, and the schema roster.
- Models live WITH their interest (`app/interests/lifting/models.rs`,
  `app/interests/spire/models.rs`, `app/analytics/models.rs`) but compile
  into the lib crate via `#[path]` re-exports in `data.rs`. That is load
  bearing: `toasty::models!(crate::*)` and the migrations CLI only see
  models reachable from the lib root. A model declared outside that tree is
  silently absent from the schema.
- Queries and import logic live with the interest too; `data.rs` is only the
  handle and the schema.
- `src/bin/migrate.rs` — the migrations CLI (toasty-cli ships as a library,
  there is no prebuilt binary).

## The handle

`Data` connects lazily behind a `OnceCell` and is cheap to clone, so a
missing or unreachable database never stops the binary from booting —
readers get `DataError` and render a stale-cache or error card. Keep that
property: don't `unwrap()` a `db()` in a page.

`Data` is registered on the router (`.app_context(data.clone())` in
`src/app.rs`) and reached from a page through topcoat's free function:

```rust
use topcoat::context::app_context;
let db = app_context::<Data>(cx).db().await?;  // Err(Unconfigured) if no POSTGRES_URL
```

Statements borrow their executor **mutably**, so the idiom everywhere is to
clone the handle first — the clone is cheap, the borrow checker is not:

```rust
pub async fn list_runs(db: &Db) -> toasty::Result<Vec<SpireRun>> {
    let mut db = db.clone();
    SpireRun::all().exec(&mut db).await
}
```

## Models

```rust
#[derive(Debug, toasty::Model)]
#[table = "sets"]
#[unique(workout_id, ordinal)]
pub struct LiftSet {
    #[key] pub id: String,          // #[key(a, b, c)] on the struct = composite
    #[index] pub workout_id: String,
    pub reps: Option<i64>,          // Option<T> = nullable column
}
```

Attributes the derive accepts: `table`, `key`, `index`, `unique`, `auto`,
`column`, `default`, `update`, `version`, `has_many`, `has_one`,
`belongs_to`. Composite forms go on the struct (`#[key(a, b, c)]`,
`#[index(kind, occurred_at)]`).

- **No relations are used here on purpose** — the readers load whole tables
  and join in Rust. Don't add `has_many` to "tidy up" a working reader.
- **Toasty hydrates whole rows.** A wide blob column is paid on every list
  read, which is why `SpireRunRaw` holds the ~100 KB `.run` payload in its
  own table, write-only by construction. New blob → new table.
- A `Json<T>` or `serde_json::Value` field **must** carry an explicit
  `#[column(type = …)]` (`text`, `varchar(n)`, `json`, `jsonb`) or it does
  not compile. No model here has one — `SpireRunRaw.raw` is a plain `String`.
- Money/weight/effort are stored as scaled integers (`weight_milli`,
  `effort_hundredths`), never floats. Keep it that way.
- Postgres here has no CHECK constraints (D1's STRICT/CHECK didn't carry
  over): the import validators are the only line of defense.

## Reads

```rust
Model::all().order_by(Model::fields().start_time().desc()).exec(&mut db)
Model::filter_by_k("version").first().exec(&mut db)          // generated per key
Model::filter(Model::fields().name().in_list(names)).exec(&mut db)

use toasty::stmt::{List, Query};
Query::<List<Model>>::all()                  // projection: Vec<String>, not Vec<Model>
    .select(Model::fields().id())
    .filter(Model::fields().id().in_list(ids))
    .exec(&mut db)
Query::<List<Model>>::all().filter(…).delete().exec(&mut tx)
```

`.limit(n)`, `.offset(n)`, and `.count()` exist. **`group_by`, `sum`,
`avg`, and joins do not** — that is the whole reason analytics drops to raw
SQL (below), and why records are derived in Rust rather than in the query.

## Writes

```rust
toasty::create!(Model { id: id.clone(), n: 1i64 }).exec(&mut tx).await?;

let mut create = Model::create_many();           // one round trip for the batch
for item in &items { create = create.item(toasty::create!(Model { … })); }
create.exec(&mut tx).await?;

let mut row = row;                               // update! needs an owned mut binding
toasty::update!(row { v: next }).exec(&mut tx).await?;
```

Transactions: `let mut tx = db.transaction().await?;` then pass `&mut tx`
to every `exec` and finish with `tx.commit().await?`. `transaction()` takes
`&mut self` precisely so you can't accidentally run a statement on the
parent handle and silently escape the transaction.

Idempotent import is the house pattern (both sync CLIs): SELECT the ids
already stored, filter the payload down to genuinely new rows, write them
plus a version bump inside one transaction. A primary-key collision aborts
the transaction, and the CLI just reruns.

Upsert exists — `Model::upsert_by_<field>(…)`, generated per primary key and
per unique constraint, with `.on_create()`, `.on_update()`, and `.or_ignore()`
(`Some(row)` on insert, `None` on conflict) — but it is one statement per row.
The import paths stay on select-then-`create_many` on purpose: `added`/
`skipped` come from the pre-select, and the batch is one round trip.

## Raw SQL escape hatch

For aggregates, CTEs, `ON CONFLICT`, window functions — anything the
builder can't express:

```rust
use toasty::Executor;

let rows: Vec<toasty::stmt::Value> = toasty::sql::query(
    "INSERT INTO t (k, v) VALUES ($1, $2)
     ON CONFLICT (k) DO UPDATE SET v = excluded.v
     RETURNING v",
)
.bind(key)
.bind(value)
.exec(&mut *executor)
.await?;
```

Postgres placeholders (`$1`), one `.bind()` per placeholder in order.
Results come back as untyped `Vec<stmt::Value>`, one record per row: reach
the columns with `Value::as_record()`, then `as_str` for text. Integers
arrive at whatever width the driver picked, so match the variants rather
than assuming `I64` — that is what `integer()` in `analytics/dashboard.rs`
is for; `one_text()` in `analytics/db.rs` is the text twin. Reuse both
instead of unpacking inline.

Write helpers against `&mut dyn Executor`, not `&mut Db`: the same function
then works on a handle or inside a transaction. `Db` and `Transaction` both
implement `Executor`.

## Migrations

Artifacts live in `toasty/` and are committed: `history.toml`,
`migrations/NNNN_name.sql`, `snapshots/NNNN_snapshot.toml`. The database
tracks what it has applied in `__toasty_migrations`. `Toasty.toml` is the
CLI defaults (sequential prefixes, no checksums, statement breakpoints).

```sh
just migrate-local migration generate --name add_x_tables   # after editing models
just migrate-local migration apply
just migrate migration apply                                 # PROD (POSTGRES_URL from .env)
```

Subcommands: `apply`, `generate`, `snapshot`, `drop`, `reset`.

- Generate reads the models and diffs against the newest snapshot — edit
  the model file first, then generate; hand-writing the SQL desyncs the
  snapshot and the next generate produces garbage.
- Review the generated SQL before applying. Toasty writes what the diff
  implies, including drops.
- `migration reset` **drops every table**. Local only.
- After a toasty version bump, run `just migrate-local migration generate`
  before touching a model: it must print "no migration needed", or a changed
  snapshot format will surface later as a bogus migration welded to a real
  schema edit.
- The runtime image has no `migrate` binary; production migrations are run
  from a checkout (`docs/railway-deploy.md`).
- Writing to prod Postgres from Claude is blocked by the permission
  classifier — Ben runs prod migrations himself (suggest `! <cmd>`).

## Gotchas

- A model not reachable from the lib crate root is invisible to
  `toasty::models!` and to migration generation, with no error.
- `exec` wants `&mut` — clone the `Db` handle per query function.
- `toasty::Result`/`toasty::Error` are re-exports of `toasty_core`; the db
  modules return `toasty::Result` and let callers map to `anyhow`.
- `toasty::query!` and `Batch`/`batch` exist but are unused here; prefer the
  patterns already in `src/app/**/db.rs` over inventing a third style.
- After any model change run `just check` — the schema is only validated
  when the CLI or a query actually runs.
