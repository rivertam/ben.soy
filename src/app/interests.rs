//! The interest page modules. Each interest is a standalone top-level page
//! (`/{slug}`) declared here; the flat `~` listing and the tmux status-bar
//! windows are its indexes now (the old `/interests` page 308s home from
//! `home.rs`).

mod drums;
mod felix;
mod keyboards;
pub(crate) mod lifting;
mod podrick;
mod puzzles;
mod simulation;
pub(crate) mod spire;
mod swing;
