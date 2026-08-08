//! The diary offline queue, written once and run on both sides.
//!
//! The site's server half (`src/app/diary.rs`) and the service worker's wasm
//! half (`crates/diary-worker`) both compile this crate, which is what keeps
//! the replay protocol from drifting: the entry lifecycle and acceptance
//! policy, wire shape, and outcome classification each have one definition.
//!
//! [`entry`] owns the domain values and policy; [`contract`] owns their wire
//! envelopes. [`store`] owns key projection and queries, written against the one [`Db`] handle type so the
//! server's real database, the native tests' `mem://`, and the device's
//! `indxdb://` all run identical code. [`eastern`] is the America/New_York
//! projection those keys (and the lifting archive's permalinks) derive from.
//! [`outbox`] is the device-local write queue. docs/diary-sync.md is the
//! architecture note.

pub mod contract;
pub mod eastern;
pub mod entry;
pub mod outbox;
pub mod placement;
pub mod store;
pub mod sync;
#[cfg(feature = "view")]
pub mod views;

/// The one handle type every side uses: the server's remote connection, the
/// tests' `mem://`, the worker's `indxdb://`. Nothing downstream knows which.
pub type Db = surrealdb::Surreal<surrealdb::engine::any::Any>;
