//! The interest page modules. Each interest is a standalone top-level page
//! (`/{slug}`) declared here; the tmux status-bar windows and home's `more`
//! listing are its indexes now (the old `/interests` page 308s home from
//! `home.rs`). felix and lifting are additionally promoted to panes of the
//! phone deck on `/`.

mod drums;
pub(crate) mod felix;
mod keyboards;
pub(crate) mod lifting;
mod podrick;
mod puzzles;
mod simulation;
pub(crate) mod spire;
mod swing;
