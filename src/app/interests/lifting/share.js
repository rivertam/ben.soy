// Progressive enhancement for the workout share block: the server always
// renders a selectable readonly <textarea>; with JavaScript the copy
// button appears and writes that text to the clipboard. Mirrors
// auto-filter.js in spirit — no framework, no state, the no-JS path
// keeps working.

const RESET_MS = 2000;

for (const button of document.querySelectorAll("[data-share] button[data-share-copy]")) {
  const container = button.closest("[data-share]");
  const box = container?.querySelector("textarea");
  if (!box || !navigator.clipboard?.writeText) continue;

  const hint = container.querySelector("[data-share-hint]");
  if (hint) hint.hidden = true;
  button.hidden = false;
  box.addEventListener("focus", () => box.select());

  const idle = button.textContent;
  let reset = 0;
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(box.value);
      button.textContent = "copied";
    } catch {
      button.textContent = "copy failed — text selected instead";
      box.focus();
    }
    clearTimeout(reset);
    reset = setTimeout(() => {
      button.textContent = idle;
    }, RESET_MS);
  });
}
