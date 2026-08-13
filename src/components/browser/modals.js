// The one modal driver. Event delegation over the whole document turns any
// native <dialog data-modal> into a working modal: showModal() supplies the
// focus trap, the inert background, Escape-to-close, and the ::backdrop, so
// this only wires opening, dismissal, and focus return by attribute. Nothing
// here touches class names — Tailwind scans .rs only, and behavior rides on
// data attributes and native dialog state (see styles/site.css `.modal*`).
//
// Contract:
//   <dialog data-modal id="…">          the surface (components::modal renders it)
//     … [data-modal-close] …            any control that dismisses it
//   [data-modal-open-on-load]           open as soon as parsed (errors/notices)
//   <a data-modal-open="id" href="…">   a trigger anywhere; the href is the
//                                        no-JS / unsupported-browser fallback
// Dialogs also emit bubbling `modal:open` / `modal:close` events, the seam a
// feature-specific companion (e.g. auth-dialog.js) hooks without re-deriving
// any of the above.

// Which element to refocus when a given dialog closes (its opener).
const openers = new WeakMap();

const canModal = (dialog) =>
  dialog instanceof HTMLDialogElement && typeof dialog.showModal === "function";

// Native showModal focuses the first autofocus/focusable child; make it
// deterministic and prefer an explicit autofocus, then the close control.
const focusInside = (dialog) => {
  const target =
    dialog.querySelector("[autofocus]") ||
    dialog.querySelector("[data-modal-close]") ||
    dialog;
  try {
    target.focus({ preventScroll: true });
  } catch {
    /* a detached or hidden target is not worth failing the open over */
  }
};

const open = (dialog, trigger) => {
  if (!canModal(dialog) || dialog.open) return false;
  if (trigger) openers.set(dialog, trigger);
  dialog.showModal();
  focusInside(dialog);
  dialog.dispatchEvent(new CustomEvent("modal:open", { bubbles: true }));
  return true;
};

document.addEventListener("click", (event) => {
  if (!(event.target instanceof Element)) return;

  const trigger = event.target.closest("[data-modal-open]");
  if (trigger) {
    // Let the browser keep new-tab / modified clicks as real navigations.
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }
    const dialog = document.getElementById(trigger.getAttribute("data-modal-open"));
    // No dialog, or a browser without showModal: don't swallow the click —
    // the trigger's own href navigates instead (progressive enhancement).
    if (!canModal(dialog)) return;
    event.preventDefault();
    open(dialog, trigger);
    return;
  }

  const closer = event.target.closest("[data-modal-close]");
  if (closer) {
    closer.closest("dialog")?.close();
    return;
  }

  // A click whose target is the dialog itself landed on the backdrop around
  // the panel (the panel and its children are separate targets).
  if (event.target instanceof HTMLDialogElement && event.target.hasAttribute("data-modal")) {
    event.target.close();
  }
});

for (const dialog of document.querySelectorAll("dialog[data-modal]")) {
  // One close handler covers every route out — button, backdrop, Escape, or a
  // companion calling close() — so focus return has a single home.
  dialog.addEventListener("close", () => {
    dialog.dispatchEvent(new CustomEvent("modal:close", { bubbles: true }));
    const opener = openers.get(dialog);
    openers.delete(dialog);
    if (opener && opener.isConnected) opener.focus();
  });
  if (dialog.hasAttribute("data-modal-open-on-load")) open(dialog, null);
}

for (const trigger of document.querySelectorAll("[data-modal-open]")) {
  const dialog = document.getElementById(trigger.getAttribute("data-modal-open"));
  if (dialog) {
    trigger.setAttribute("aria-haspopup", "dialog");
    trigger.setAttribute("aria-controls", dialog.id);
  }
}
