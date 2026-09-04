//! The lifting archive engine and the small Health Connect step ledger — the
//! Eastern projection, derived-records spec, import validation/write paths,
//! filter grammar, and the in-memory snapshot that serves lifting reads.
//!
//! Reads come from a [`snapshot`] of the whole archive rather than
//! per-request SQL: records are derived-not-stored (a `has_record` filter
//! that participates in counts and pagination has no SQL expression once
//! the `set_records` table is gone), and every SQLite-ism the old Worker
//! relied on (ASCII-only NOCASE, byte-order sorts, NULL-excluding
//! comparisons) has an exact pure-Rust mirror here, independent of datastore
//! collation. The database is read in full only when the data version changes
//! — an import, a few times a week.
//!
//! Error messages, envelope key order, and filter semantics are contract:
//! the golden fixtures under `tests/fixtures/api` capture the Worker's
//! originals verbatim. The sibling `models.rs` belongs to this interest
//! too but compiles inside the lib crate (see `src/data.rs`), and
//! `fitness_sync.rs` is its own binary (`Cargo.toml` `[[bin]]`).

pub(crate) mod aliases;
pub(crate) mod api;
pub(crate) mod db;
// The lightweight projection is shared by the native server and both wasm
// workers without making Fitness entry pull in Diary's database engine.
pub(crate) use eastern_time as eastern;
pub(crate) mod filters;
pub(crate) mod import;
pub(crate) mod manual;
pub(crate) mod native_entry;
pub(crate) mod records;
pub(crate) mod routes;
pub(crate) mod scoring;
pub(crate) mod snapshot;
pub(crate) mod steps;
pub(crate) mod store;
mod validate;
