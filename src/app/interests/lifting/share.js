// The text, image preview, and PNG download work without JavaScript.
// Clipboard copying is an enhancement of those same server-rendered values.

const RESET_MS = 2000;

for (const container of document.querySelectorAll("[data-share]")) {
  const box = container.querySelector("textarea");
  const textButton = container.querySelector("[data-share-copy]");
  const imageButton = container.querySelector("[data-share-copy-image]");
  const hint = container.querySelector("[data-share-hint]");
  const status = container.querySelector("[data-share-status]");
  const imageUrl = container.dataset.shareImage;
  const buttons = [textButton, imageButton].filter(Boolean);
  const idleLabels = new Map(buttons.map((button) => [button, button.textContent]));
  let busy = false;
  let reset = 0;

  const announce = (message) => {
    if (status) status.textContent = message;
  };
  const copy = async (button, write, success, failure) => {
    if (busy) return;
    busy = true;
    clearTimeout(reset);
    for (const action of buttons) {
      action.disabled = true;
      action.textContent = idleLabels.get(action);
    }
    const idle = idleLabels.get(button);
    button.textContent = "copying…";
    announce("");
    try {
      // Invoke write before the first await, within the click gesture.
      await write();
      button.textContent = "copied";
      announce(success);
    } catch {
      button.textContent = idle;
      failure();
    } finally {
      busy = false;
      for (const action of buttons) action.disabled = false;
      reset = setTimeout(() => {
        button.textContent = idle;
      }, RESET_MS);
    }
  };

  if (box && textButton && navigator.clipboard?.writeText) {
    if (hint) hint.hidden = true;
    textButton.hidden = false;
    box.addEventListener("focus", () => box.select());
    textButton.addEventListener("click", () => copy(
      textButton,
      () => navigator.clipboard.writeText(box.value),
      "Workout text copied.",
      () => {
        box.focus();
        box.select();
        announce("Copying was blocked. The text is selected so you can copy it manually.");
      },
    ));
  }

  if (imageButton && imageUrl && navigator.clipboard?.write
      && typeof ClipboardItem === "function"
      && (typeof ClipboardItem.supports !== "function" || ClipboardItem.supports("image/png"))) {
    imageButton.hidden = false;
    imageButton.addEventListener("click", () => copy(
      imageButton,
      () => {
        const png = fetch(imageUrl, { credentials: "same-origin" }).then(async (response) => {
          if (!response.ok) throw new Error("Image unavailable");
          const blob = await response.blob();
          if (blob.type !== "image/png" || !blob.size) throw new Error("Invalid image");
          return blob;
        });
        // Clipboard access may reject before the image request finishes.
        png.catch(() => {});
        // Safari requires write() during the gesture; defer only the PNG data.
        // https://webkit.org/blog/10855/async-clipboard-api/
        return navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
      },
      "Workout image copied.",
      () => announce("Couldn't copy the image. Use “save image” to share the PNG."),
    ));
  }
}
