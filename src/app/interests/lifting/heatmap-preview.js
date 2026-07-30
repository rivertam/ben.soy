// Progressive enhancement for the heatmap day popover. The Topcoat signal
// (hidden input) selects which day the preview shard loads; this script only
// opens/pins/hides the shared popover and keeps it CSS-anchored to the active
// day cell. Without it, click still opens via native `popovertarget` (which
// sets the implicit invoker/anchor itself).

const HIDE_MS = 1000;
const root = document.querySelector("[data-heatmap-previews]");

if (root && typeof HTMLElement !== "undefined" && "showPopover" in HTMLElement.prototype) {
  const input = root.querySelector("[data-heatmap-day-input]");
  const panel = root.querySelector("[data-heatmap-panel]");
  if (!(input instanceof HTMLInputElement) || !(panel instanceof HTMLElement)) {
    // Missing chrome — leave native popovertarget alone.
  } else if (typeof panel.showPopover === "function") {
    let hideTimer = 0;

    const clearHide = () => {
      window.clearTimeout(hideTimer);
      hideTimer = 0;
    };

    const setDay = (date) => {
      panel.style.setProperty("position-anchor", `--heatmap-day-${date}`);
      if (input.value === date) return;
      input.value = date;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    };

    const clearDay = () => {
      panel.style.removeProperty("position-anchor");
      if (input.value === "") return;
      input.value = "";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    };

    const open = (button) => {
      if (panel.matches(":popover-open")) return;
      try {
        panel.showPopover({ source: button });
      } catch {
        panel.showPopover();
      }
    };

    const show = (date, button) => {
      clearHide();
      setDay(date);
      open(button);
    };

    const scheduleHide = () => {
      clearHide();
      hideTimer = window.setTimeout(() => {
        hideTimer = 0;
        if (panel.dataset.pinned === "true") return;
        if (panel.matches(":popover-open")) panel.hidePopover();
        clearDay();
      }, HIDE_MS);
    };

    for (const button of root.querySelectorAll("[data-heatmap-trigger]")) {
      if (!(button instanceof HTMLElement)) continue;
      const date = button.dataset.heatmapDate;
      if (!date) continue;

      button.addEventListener("mouseenter", () => show(date, button));
      button.addEventListener("mouseleave", scheduleHide);
      button.addEventListener("click", () => {
        clearHide();
        setDay(date);
        panel.dataset.pinned = "true";
        open(button);
      });
    }

    panel.addEventListener("mouseenter", clearHide);
    panel.addEventListener("mouseleave", scheduleHide);

    panel.addEventListener("toggle", (event) => {
      if (event.newState === "closed") {
        delete panel.dataset.pinned;
        clearHide();
        clearDay();
      }
    });
  }
}
