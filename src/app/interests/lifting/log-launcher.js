// Keep the native <details> action tray open while its modal is active, then
// return to the originating "log" control after every dismissal path. The
// generic modal driver owns opening, Escape/backdrop dismissal, and focus
// return.

const dialogIds = new Set([
  "fitness-lift-dialog",
  "fitness-run-dialog",
  "fitness-interruption-dialog",
]);
const launchers = new WeakMap();
const launcherElements = document.querySelectorAll("[data-fitness-log-launcher]");

// `details[name]` gives supporting browsers native accordion behavior. Keep
// the same invariant in the companion so two phone panes can never retain
// open trays after a swipe between them.
for (const launcher of launcherElements) {
  launcher.addEventListener("toggle", () => {
    if (!launcher.open) return;
    for (const peer of launcherElements) {
      if (peer !== launcher) peer.open = false;
    }
  });
}

// Home has matching controls in its log and fitness panes. Remember which
// tray launched each dialog so dismissal returns to that exact header rather
// than whichever launcher happened to appear first in the document.
document.addEventListener(
  "click",
  (event) => {
    if (!(event.target instanceof Element)) return;
    const trigger = event.target.closest("[data-modal-open]");
    if (!trigger) return;
    const dialog = document.getElementById(trigger.getAttribute("data-modal-open"));
    const launcher = trigger.closest("[data-fitness-log-launcher]");
    if (
      dialog instanceof HTMLDialogElement &&
      dialogIds.has(dialog.id) &&
      launcher instanceof HTMLDetailsElement
    ) {
      launchers.set(dialog, launcher);
    }
  },
  true,
);

document.addEventListener("modal:close", (event) => {
  if (!(event.target instanceof HTMLDialogElement)) return;
  if (!dialogIds.has(event.target.id)) return;
  const launcher = launchers.get(event.target);
  launchers.delete(event.target);
  if (!(launcher instanceof HTMLDetailsElement)) return;

  // The generic driver first returns focus to the option that opened the
  // dialog. Close the tray one microtask later and leave focus on its summary,
  // which remains visible.
  queueMicrotask(() => {
    launcher.open = false;
    launcher.querySelector("summary")?.focus({ preventScroll: true });
  });
});
