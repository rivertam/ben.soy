// Enhance the ordinary target form with native, keyboard-accessible sliders.
// Only the exact-value field has a name: unset goals still submit as blank,
// and a slider at zero remains an explicit zero. Nothing saves until submit.
const form = document.querySelector("[data-muscle-targets]");

if (form instanceof HTMLFormElement) {
  const rows = [...form.querySelectorAll("[data-target-row]")].map((row) => ({
    row,
    field: row.querySelector("[data-target-value]"),
    range: row.querySelector("[data-target-range]"),
    slider: row.querySelector("[data-target-slider]"),
    actions: row.querySelector("[data-target-actions]"),
    clear: row.querySelector("[data-target-clear]"),
    state: row.querySelector("[data-target-state]"),
    error: row.querySelector("[data-target-error]"),
    recent: Number(row.dataset.recentScaled) / 800,
    usual: Number(row.dataset.usualScaled) / 800,
  }));
  const scaleLabel = form.querySelector("[data-target-scale]");

  function targetValue(value) {
    const text = value.trim();
    if (!text || !/^\d+(?:\.\d)?$/.test(text)) return null;
    const points = Number(text);
    return Number.isFinite(points) && points <= 10000 ? points : null;
  }

  // One shared scale keeps comparisons meaningful across all muscle rows.
  // Leave room to raise goals, and expand on exact input rather than moving
  // the scale underneath a drag. The 10,000-point storage limit is unchanged.
  function scaleFor(value) {
    return Math.min(10000, Math.max(50, Math.ceil(value * 1.5 / 10) * 10));
  }
  let maximum = scaleFor(Math.max(0, ...rows.flatMap((item) => [
    item.recent, item.usual, targetValue(item.field.value) ?? 0,
  ])));

  function paintScale() {
    for (const item of rows) {
      item.range.max = String(maximum);
      item.row.style.setProperty("--target-recent-width", `${Math.min(100, item.recent / maximum * 100)}%`);
      item.row.style.setProperty("--target-usual-left", `${Math.min(100, item.usual / maximum * 100)}%`);
    }
    scaleLabel.textContent = `0–${maximum} points`;
    scaleLabel.hidden = false;
  }

  function sync(item) {
    const points = targetValue(item.field.value);
    const empty = !item.field.value.trim();
    const invalid = !empty && points === null;
    if (points !== null && points > maximum) {
      maximum = scaleFor(points);
      paintScale();
    }
    item.range.value = String(points ?? Math.min(maximum, Math.round(item.usual * 10) / 10));
    item.range.setAttribute("aria-valuetext", points === null
      ? "No target set. Slide to set weekly points."
      : `${points} weekly points`);
    item.row.dataset.targetSet = String(points !== null);
    item.field.setAttribute("aria-invalid", String(invalid));
    item.state.textContent = invalid ? "Check target value" : empty ? "Using usual pace" : `${points} points / week`;
    item.clear.disabled = empty;
    if (item.error) item.error.hidden = !invalid;
  }

  paintScale();
  for (const item of rows) {
    sync(item);
    item.range.addEventListener("input", () => {
      item.field.value = item.range.value;
      sync(item);
    });
    item.field.addEventListener("input", () => sync(item));
    item.clear.addEventListener("click", () => {
      item.field.value = "";
      sync(item);
      item.range.focus();
    });
    item.slider.hidden = false;
    item.actions.hidden = false;
  }

  // Browser back/forward can restore exact input values after script setup.
  window.addEventListener("pageshow", () => rows.forEach(sync));
}
