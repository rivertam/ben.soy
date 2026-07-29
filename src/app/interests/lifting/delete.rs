//! The admin-only delete control on a workout page.
//!
//! It calls `DELETE /api/fitness/workouts/by-path/{path}` — the same resource
//! `just delete-lift` uses, authorized by the viewer cookie rather than the
//! sync token (`docs/fitness.md`). The archive stays create-only everywhere
//! else; correcting a lift means deleting it and publishing it again.
//!
//! Rendered `hidden` and revealed by `delete-lift.js`, like the share block's
//! copy button. That is not decoration: a form cannot issue DELETE, so a
//! visible-but-inert button is the one thing this control must never be.

use topcoat::{
    Result,
    asset::{Asset, asset},
    view::{component, view},
};

use super::format::plural;

pub(super) const DELETE_LIFT_JS: Asset = asset!("./delete-lift.js");

// Tailwind vocab for the delete control. Utilities stay whole per line for
// the build-time class scanner; the script only toggles `hidden` and text, so
// no class name here is reachable from JavaScript alone.
const DELETE_SECTION: &str = "mt-10 pt-4 border-t border-hairline text-right";
const DELETE_START: &str = "quiet-link cursor-pointer font-meta text-xs";
const DELETE_COPY: &str = "font-meta text-xs leading-[1.6] text-ink2";
const DELETE_ACTIONS: &str = "mt-3 flex flex-wrap items-center justify-end gap-3";
const DELETE_COMMIT: &str = "px-3 py-[0.45rem] font-meta text-[0.7rem] text-card bg-oxide \
     border border-oxide rounded-[0.2rem] cursor-pointer hover:text-white hover:bg-oxide-hot \
     hover:border-oxide-hot focus-visible:text-white focus-visible:bg-oxide-hot \
     focus-visible:border-oxide-hot disabled:cursor-not-allowed disabled:opacity-60";
const DELETE_CANCEL: &str = "quiet-link cursor-pointer font-meta text-xs";
const DELETE_STATUS: &str = "mt-2 font-meta text-[0.67rem] leading-[1.5] text-muted";

/// `set_count` rides along so the confirmation names what is about to go.
/// Records are derived, so they need no mention: the remaining history
/// re-derives its own podium.
#[component]
pub(super) async fn delete_control(path: &str, set_count: usize) -> Result {
    let sets = format!("{set_count} {}", plural(set_count, "set", "sets"));
    view! {
        <section
            class=(DELETE_SECTION)
            data-lift-delete=""
            data-lift-delete-path=(path)
            aria-label="Delete this lift"
            hidden=""
        >
            <button type="button" class=(DELETE_START) data-lift-delete-start="">
                "delete this lift"
            </button>
            <div data-lift-delete-confirm="" hidden="">
                <p class=(DELETE_COPY)>
                    (format!("Permanently delete this workout and its {sets}?"))
                    " A pasted workout is not in the CSV and cannot be resynced."
                </p>
                <div class=(DELETE_ACTIONS)>
                    <button type="button" class=(DELETE_CANCEL) data-lift-delete-cancel="">
                        "cancel"
                    </button>
                    <button type="button" class=(DELETE_COMMIT) data-lift-delete-commit="">
                        "delete permanently"
                    </button>
                </div>
            </div>
            <p class=(DELETE_STATUS) data-lift-delete-status="" role="status" aria-live="polite"></p>
        </section>
    }
}

#[cfg(test)]
mod tests {
    /// The control is inert without its script, so the script must be the
    /// thing that reveals it — never CSS, never a server-side `hidden` that
    /// something else clears.
    #[test]
    fn the_script_reveals_the_control_and_calls_the_rest_resource() {
        let source = include_str!("./delete-lift.js");
        assert!(source.contains("panel.hidden = false"));
        assert!(source.contains(r#"method: "DELETE""#));
        assert!(source.contains("/api/fitness/workouts/by-path/"));
        assert!(source.contains(r#"credentials: "same-origin""#));
        // Two steps, and the safe control takes focus between them.
        assert!(source.contains("cancel.focus()"));
        assert!(source.contains("window.location.assign(\"/lifting\")"));
    }
}
