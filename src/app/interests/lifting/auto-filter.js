// Progressive enhancement for the log filter chrome.
// Native popovers + GET links/forms work without this; the script only
// swaps the details fallback for the compact "+ filter" button.

const root = document.querySelector("[data-lifting-filters]");
const addButton = document.querySelector("[data-lifting-add]");
const fallback = document.querySelector("[data-lifting-filters-fallback]");

if (
  root instanceof HTMLElement &&
  addButton instanceof HTMLButtonElement &&
  fallback instanceof HTMLDetailsElement
) {
  fallback.hidden = true;
  addButton.hidden = false;

  // Opening a value panel should dismiss the category list. `popover=auto`
  // already allows only one light-dismiss popover in supporting browsers;
  // this is a belt-and-braces close when the category button also carries
  // an explicit close target.
  root.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const opener = target.closest("[data-lifting-category]");
    if (!(opener instanceof HTMLElement)) return;
    const closeId = opener.dataset.liftingClose;
    if (!closeId) return;
    const panel = document.getElementById(closeId);
    if (panel instanceof HTMLElement && typeof panel.hidePopover === "function") {
      if (panel.matches(":popover-open")) panel.hidePopover();
    }
  });

  // Focus the first field when a value popover opens.
  for (const panel of root.querySelectorAll("[data-lifting-value]")) {
    if (!(panel instanceof HTMLElement)) continue;
    panel.addEventListener("toggle", (event) => {
      if (!(event.target instanceof HTMLElement)) return;
      if (!event.target.matches(":popover-open")) return;
      const field = event.target.querySelector("input, select");
      if (field instanceof HTMLElement) field.focus();
    });
  }
}
