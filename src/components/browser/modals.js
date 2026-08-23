// The shared overlay driver. Event delegation over the whole document turns
// native dialogs and inline citation popovers into working controls. Nothing
// here touches class names — Tailwind scans .rs only, and behavior rides on
// data attributes and native element state.
//
// Dialog contract:
//   <dialog data-modal id="…">          the surface (components::modal renders it)
//     … [data-modal-close] …            any control that dismisses it
//   [data-modal-open-on-load]           open as soon as parsed (errors/notices)
//   <a data-modal-open="id" href="…">   a trigger anywhere; the href is the
//                                        no-JS / unsupported-browser fallback
// Dialogs also emit bubbling `modal:open` / `modal:close` events, the seam a
// feature-specific companion (e.g. auth-dialog.js) hooks without re-deriving
// any of the above.
//
// Inline-popover contract:
//   <a data-inline-popover-trigger="id"> a genuinely inline, wrapping trigger
//   <span id="id" popover>                the server-rendered popover
// Native `popovertarget` only works on atomic form controls. The small driver
// lets prose use a fragmenting anchor while retaining native light-dismiss,
// Escape, and top-layer behavior on the popover itself.

// Which element to refocus when a given dialog closes (its opener).
const openers = new WeakMap();

const canPopover = (popover) =>
  popover instanceof HTMLElement &&
  typeof popover.showPopover === "function" &&
  typeof popover.hidePopover === "function";

const popoverFor = (trigger) =>
  document.getElementById(trigger.getAttribute("data-inline-popover-trigger"));

const setPopoverExpanded = (popover) => {
  const expanded = popover.matches(":popover-open") ? "true" : "false";
  for (const trigger of document.querySelectorAll(
    `[data-inline-popover-trigger="${CSS.escape(popover.id)}"]`,
  )) {
    trigger.setAttribute("aria-expanded", expanded);
  }
};

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

  const popoverTrigger = event.target.closest("[data-inline-popover-trigger]");
  if (popoverTrigger) {
    const popover = popoverFor(popoverTrigger);
    if (!canPopover(popover)) return;
    event.preventDefault();
    if (popover.matches(":popover-open")) {
      popover.hidePopover();
    } else {
      // Open after the activating click finishes. Otherwise that same click
      // is also interpreted as an outside click against the newly open panel.
      setTimeout(() => {
        if (popover.isConnected && !popover.matches(":popover-open")) {
          popover.showPopover({ source: popoverTrigger });
        }
      }, 0);
    }
    return;
  }

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

document.addEventListener("keydown", (event) => {
  if (event.key !== " " || !(event.target instanceof Element)) return;
  const trigger = event.target.closest("[data-inline-popover-trigger]");
  if (!trigger) return;
  event.preventDefault();
  trigger.click();
});

for (const popover of document.querySelectorAll("[popover]")) {
  if (!popover.id) continue;
  popover.addEventListener("toggle", () => setPopoverExpanded(popover));
  setPopoverExpanded(popover);
}

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
