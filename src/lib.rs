//! Shared data-layer library.
//!
//! The site binary (`src/main.rs`) keeps its rendering modules private; this
//! crate holds the logic shared across the server and the sync CLIs.

pub mod auth;
pub mod data;
