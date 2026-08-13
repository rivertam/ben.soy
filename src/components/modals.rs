//! Generic modal dialogs.
//!
//! A native `<dialog>` opened with `showModal()` already provides the defining
//! modal behaviors — a focus trap, an inert background, Escape-to-close, and a
//! `::backdrop` — so the shared `modals.js` (chrome.rs links it once) only wires
//! opening, dismissal, and focus return by attribute. Authors write Rust plus
//! classes: render `modal(...)` for the surface and mark any trigger with
//! `data-modal-open="<id>"`. Triggers stay real links, so without JavaScript or
//! dialog support the href still navigates — the modal is a pure enhancement.

use topcoat::{
    Result,
    view::{View, component, view},
};

/// A focus-trapping modal dialog opened from anywhere by
/// `data-modal-open="<id>"`.
///
/// `id` must be unique on the page. `label` is the small frame caption over the
/// border. `labelledby`, when set, is the id of the heading inside `child` that
/// names the dialog for assistive tech; left empty, `label` becomes the
/// accessible name. `open_on_load` lets the server request that the dialog open
/// as soon as it is parsed — for a returned error or one-shot notice.
///
/// The surface ships a close control and the panel wrapper; `child` is the
/// dialog body. Styling lives in `styles/site.css` under `.modal*`.
#[component]
pub async fn modal(
    id: &str,
    label: &str,
    #[default("")] labelledby: &str,
    #[default(false)] open_on_load: bool,
    child: View,
) -> Result {
    view! {
        <dialog
            id=(id)
            class="modal"
            data-modal=""
            data-modal-open-on-load=(open_on_load.then_some(""))
            aria-labelledby=((!labelledby.is_empty()).then_some(labelledby))
            aria-label=(labelledby.is_empty().then_some(label))
        >
            <span class="modal-label" aria-hidden="true">(label)</span>
            <div class="modal-panel">
                <button
                    type="button"
                    class="modal-close"
                    data-modal-close=""
                    aria-label="Close dialog"
                >"×"</button>
                (child)
            </div>
        </dialog>
    }
}
