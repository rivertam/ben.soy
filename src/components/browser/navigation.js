// Site-wide structural keyboard navigation. Appearance does not participate:
// f follows visible controls, while j/k move through intrinsic rail rows and
// explicitly adapted list rows in document order.

const config = document.querySelector("[data-navigation-runtime]");

let hints = null;
let hintsPromise = null;
const hintFeatures = () => {
  hintsPromise ||= import(config.dataset.hintsModule).then((hintModule) => {
    hints = hintModule.createHints();
    return hints;
  });
  return hintsPromise;
};

const editable = (target) =>
  target?.closest("input, textarea, select, [contenteditable='true']");

const railItems = () =>
  [...document.querySelectorAll(".rail-row, [data-rail-item]")].filter(
    (item) =>
      !item.hasAttribute("data-rail-ignore") &&
      !item.closest("[hidden]") &&
      item.getClientRects().length > 0
  );

let currentRail = null;

const clearRail = () => {
  const hadCurrent = !!currentRail;
  document.querySelectorAll("[data-rail-current]").forEach((item) => {
    item.removeAttribute("data-rail-current");
  });
  currentRail = null;
  return hadCurrent;
};

const selectRail = (index) => {
  const items = railItems();
  if (!items.length) return false;
  const selected = items[Math.max(0, Math.min(items.length - 1, index))];
  clearRail();
  currentRail = selected;
  selected.dataset.railCurrent = "";
  selected.scrollIntoView({ block: "nearest" });
  return true;
};

const moveRail = (delta) => {
  const items = railItems();
  if (!items.length) return false;
  const at = items.indexOf(currentRail);
  if (at < 0) return selectRail(delta > 0 ? 0 : items.length - 1);
  return selectRail(at + delta);
};

const activateRail = () => {
  if (!currentRail || !currentRail.isConnected) return false;
  const control = currentRail.matches("[data-rail-enter]")
    ? currentRail
    : currentRail.querySelector("[data-rail-enter]");
  if (control && !control.disabled) {
    control.click();
    return true;
  }
  const href = currentRail.dataset.railHref;
  if (!href) return false;
  location.href = href;
  return true;
};

const acknowledge = (key) => {
  document.dispatchEvent(
    new CustomEvent("site:navigationkey", { detail: { key } })
  );
};

let lastG = 0;
document.addEventListener("keydown", (event) => {
  const target = event.target instanceof Element ? event.target : null;
  if (editable(target)) return;
  if (hints?.isActive()) {
    hints.key(event);
    return;
  }
  if (event.ctrlKey || event.altKey || event.metaKey) return;

  switch (event.key) {
    case "j":
      acknowledge(event.key);
      if (!moveRail(1)) scrollBy({ top: 80, behavior: "instant" });
      event.preventDefault();
      break;
    case "k":
      acknowledge(event.key);
      if (!moveRail(-1)) scrollBy({ top: -80, behavior: "instant" });
      event.preventDefault();
      break;
    case "f":
      acknowledge(event.key);
      event.preventDefault();
      void hintFeatures().then((loadedHints) => loadedHints.start());
      break;
    case "G":
      if (!selectRail(railItems().length - 1)) {
        scrollTo({
          top: document.documentElement.scrollHeight,
          behavior: "instant",
        });
      }
      event.preventDefault();
      break;
    case "g":
      if (performance.now() - lastG < 450) {
        if (!selectRail(0)) scrollTo({ top: 0, behavior: "instant" });
        event.preventDefault();
      }
      lastG = performance.now();
      break;
    case "Enter":
      if (target?.closest("a, button, summary, [tabindex]")) return;
      if (activateRail()) event.preventDefault();
      break;
    case "Escape":
      if (clearRail()) event.preventDefault();
      break;
  }
});
