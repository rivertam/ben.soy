// Progressive enhancement for the admin-only workout dialog. One click reads
// Lyfta's copied text, publishes it, and replaces the clipboard with the same
// server-rendered share text exposed on the resulting workout page.
//
// ClipboardItem accepts a promised Blob. Calling clipboard.write() during the
// click, before the upload finishes, preserves WebKit's user-gesture grant
// while the promise waits for the server. The ordinary textarea + form remain
// usable when clipboard APIs or permissions are unavailable.

// Opening, Escape/backdrop dismissal, the focus trap, and focus return belong
// to the site's generic native-dialog driver. This companion only owns the
// Lyfta clipboard workflow inside the lift dialog.
const dialog = document.querySelector("#fitness-lift-dialog");
const form = dialog?.querySelector("form[data-workout-upload]");
const box = form?.querySelector("textarea[name=workout]");
const label = form?.querySelector("[data-workout-upload-label]");
const upload = form?.querySelector("[data-workout-upload-clipboard]");
const submit = form?.querySelector("[data-workout-upload-submit]");
const copy = form?.querySelector("[data-workout-upload-copy]");
const open = form?.querySelector("[data-workout-upload-result-open]");
const status = form?.querySelector("[data-workout-upload-status]");

let published = null;

function announce(message) {
  if (status) status.textContent = message;
}

function readyForClipboardUpload() {
  return Boolean(
    window.isSecureContext
      && navigator.clipboard?.readText
      && (navigator.clipboard.write || navigator.clipboard.writeText),
  );
}

async function publish(text) {
  const body = new URLSearchParams();
  body.set("workout", text);
  const response = await fetch(form.action, {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error",
    headers: {
      "Accept": "application/json",
      "Content-Type": "application/x-www-form-urlencoded;charset=UTF-8",
    },
    body,
  });
  const result = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(result?.error || "The workout could not be published.");
  }
  if (
    typeof result?.location !== "string"
      || !/^\/fitness\/lift\/[A-Za-z0-9-]+$/.test(result.location)
  ) {
    throw new Error("The workout was published, but its page could not be opened.");
  }
  return result;
}

function showPublishedFallback(result, message) {
  published = result;
  upload.hidden = true;
  submit.hidden = true;
  box.readOnly = true;
  open.href = result.location;
  open.hidden = false;

  if (typeof result.share_text === "string") {
    box.value = result.share_text;
    label.textContent = "Published workout share text";
    copy.hidden = false;
    announce(`${message} Tap “Copy share text and open workout” to finish.`);
  } else {
    announce(`${message} Open the workout and use “share this workout” to copy it.`);
  }
}

if (
  form && box && label && upload && submit && copy && open && status
  && readyForClipboardUpload()
) {
  upload.hidden = false;

  upload.addEventListener("click", async () => {
    upload.disabled = true;
    submit.disabled = true;
    announce("Reading the Lyfta workout and publishing it…");

    // Start the read inside the click handler. It may display the browser's
    // native Paste permission UI on Safari and Firefox.
    let resultPromise;
    try {
      resultPromise = navigator.clipboard.readText().then((text) => {
        if (!text.trim()) throw new Error("The clipboard does not contain workout text.");
        box.value = text;
        return publish(text);
      });
    } catch {
      announce("Clipboard access was blocked. Paste into the text box instead.");
      upload.disabled = false;
      submit.disabled = false;
      box.focus();
      return;
    }

    // Start the write in the same gesture too. The promised Blob does not
    // resolve until the server returns the canonical site share text.
    let copyPromise = null;
    if (navigator.clipboard.write && typeof ClipboardItem === "function") {
      try {
        const shareBlob = resultPromise.then((result) => {
          if (typeof result.share_text !== "string") {
            throw new Error("Share text is temporarily unavailable.");
          }
          return new Blob([result.share_text], { type: "text/plain" });
        });
        copyPromise = navigator.clipboard.write([
          new ClipboardItem({ "text/plain": shareBlob }),
        ]);
        // The upload may reject before we reach the await below.
        copyPromise.catch(() => {});
      } catch {
        copyPromise = null;
      }
    }

    try {
      const result = await resultPromise;
      if (!copyPromise && typeof result.share_text === "string") {
        // Older engines lack promised ClipboardItem data. This succeeds in
        // browsers with a persistent clipboard-write grant and otherwise
        // falls through to the explicit second-tap copy button.
        copyPromise = navigator.clipboard.writeText(result.share_text);
      }
      if (!copyPromise) {
        showPublishedFallback(result, "The workout is published, but its share text was not copied.");
        return;
      }
      try {
        await copyPromise;
        announce("Published. The site share text is now on your clipboard.");
        window.location.assign(result.location);
      } catch {
        showPublishedFallback(result, "The workout is published, but the browser blocked clipboard replacement.");
      }
    } catch (error) {
      announce(error instanceof Error ? error.message : "The workout could not be published.");
      upload.disabled = false;
      submit.disabled = false;
      box.focus();
    }
  });

  copy.addEventListener("click", async () => {
    if (!published || typeof published.share_text !== "string") return;
    try {
      await navigator.clipboard.writeText(published.share_text);
      window.location.assign(published.location);
    } catch {
      box.focus();
      box.select();
      announce("Copy was blocked. The share text is selected; use your device's Copy command.");
    }
  });
}
