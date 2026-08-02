//! The diary offline queue, written once and run on both sides.
//!
//! The site's server half (`src/app/diary.rs`) and the service worker's wasm
//! half (`crates/diary-worker`) both compile this crate, which is what keeps
//! the replay protocol from drifting: the wire shape, the validation rules,
//! and the outcome classification have exactly one definition.
//!
//! [`contract`] is that protocol. [`outbox`] is the device-local write queue,
//! stored in a SurrealDB reached through the same `Surreal<Any>` handle the
//! server uses for the real database — on the phone that handle points at
//! `indxdb://` (IndexedDB), in tests at `mem://`, and nothing in the queue
//! logic knows or cares which. docs/diary-sync.md is the architecture note.

pub mod contract;
pub mod outbox;
