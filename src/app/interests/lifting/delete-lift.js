// The admin's delete control on a workout page. The server renders it hidden
// because there is no no-JavaScript path: a form cannot issue DELETE, and the
// archive's delete verb is DELETE on the workout resource. Without this
// script the control never appears — `just delete-lift` is the fallback.
//
// Two steps on purpose. The first click only reveals what is about to be
// destroyed; the second commits. No window.confirm(), which is unstyleable
// and blocks the page.

const panel = document.querySelector("[data-lift-delete]");
const start = panel?.querySelector("[data-lift-delete-start]");
const confirm = panel?.querySelector("[data-lift-delete-confirm]");
const commit = panel?.querySelector("[data-lift-delete-commit]");
const cancel = panel?.querySelector("[data-lift-delete-cancel]");
const status = panel?.querySelector("[data-lift-delete-status]");
const path = panel?.dataset.liftDeletePath;

function announce(message) {
  if (status) status.textContent = message;
}

function reset() {
  confirm.hidden = true;
  start.hidden = false;
  commit.disabled = false;
  announce("");
  start.focus();
}

// The API's error bodies are terse contract strings ("not found"), not
// sentences to show someone. Say what actually happened instead, and never
// claim "nothing was deleted" for a 404 — something was, just not by this
// click.
function failureMessage(status) {
  if (status === 404) return "This lift is already gone. Reload the page.";
  if (status === 401) return "Your sign-in expired. Sign in again, then retry.";
  if (status === 403) return "That request did not look same-origin. Reload the page and retry.";
  return "The lift could not be deleted, so nothing was deleted. Try again in a moment.";
}

if (panel && start && confirm && commit && cancel && status && path) {
  panel.hidden = false;

  start.addEventListener("click", () => {
    start.hidden = true;
    confirm.hidden = false;
    // Focus lands on cancel, not the destructive button: a stray Enter
    // after the first click must not delete the workout.
    cancel.focus();
  });

  cancel.addEventListener("click", reset);

  commit.addEventListener("click", async () => {
    commit.disabled = true;
    announce("Deleting…");
    try {
      const response = await fetch(
        `/api/fitness/workouts/by-path/${encodeURIComponent(path)}`,
        {
          method: "DELETE",
          credentials: "same-origin",
          cache: "no-store",
          redirect: "error",
          headers: { Accept: "application/json" },
        },
      );
      if (!response.ok) {
        announce(failureMessage(response.status));
        commit.disabled = false;
        return;
      }
      // The archive is gone from under this page, so do not re-render it —
      // /fitness shows whatever is newest now.
      announce("Deleted. Opening the latest lift…");
      window.location.assign("/fitness");
    } catch {
      // A transport failure means the request may or may not have landed.
      announce("The delete could not be sent. Reload the page to see where it stands.");
      commit.disabled = false;
    }
  });
}
