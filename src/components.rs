//! The design system: shared chrome and reusable page components.
//!
//! The look is "mill and oxide": cool steel paper, iron ink, one accent — the
//! color literally named rust. The signature is the margin rail (`.rail-row` /
//! `.rail-stamp` in `styles/site.css`): a narrow left column of Fira Mono
//! metadata, like the stamped margin of an engineering logbook.
//!
//! One file per concern; everything re-exports flat, so pages import
//! `crate::components::{shell, rail_section, …}`.

mod cards;
pub(crate) mod chrome;
mod links;
mod modals;
mod popover;
mod rail;
mod spread;

pub use cards::{index_card, video_card};
pub use chrome::{SITE_CSS, shell};
pub use links::{ext_link, link_label};
pub use modals::modal;
pub use popover::inline_popover;
pub use rail::{back_link, page_head, rail_group, rail_prose, rail_section};
pub use spread::{doc_head, full_bleed, margin_notes, stamp_seal};
