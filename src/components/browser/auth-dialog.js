// Feature companion to the generic modal driver (modals.js). That script owns
// opening the account dialog, the focus trap, and dismissal; this only keeps
// the sign-in destinations aimed at wherever the visitor is right now. The
// server seeds each `next` with the request path; the browser adds the URL
// fragment — which never reaches the server — every time the dialog opens.

const config = document.querySelector("[data-account-config]");
const dialog = config && config.closest("dialog");

if (config && dialog) {
  const fallback = config.dataset.authReturn || "/";
  const errorParam = config.dataset.authErrorParam;
  const google = dialog.querySelector("[data-auth-google]");
  const logout = dialog.querySelector("form[data-auth-logout]");

  // Where to send the visitor back to: the live document, minus the one-shot
  // popup-error param, which is never part of a return target.
  const currentReturnTarget = () => {
    const url = new URL(location.href);
    if (errorParam) url.searchParams.delete(errorParam);
    const target = `${url.pathname}${url.search}${url.hash}`;
    return target.startsWith("/") ? target : fallback;
  };

  const withNext = (raw, next) => {
    const url = new URL(raw, location.origin);
    url.searchParams.set("next", next);
    return url.href;
  };

  // Recompute on each open so a fragment the visitor navigated to (or a query
  // that changed) rides along. `set` replaces, so repeated opens don't stack.
  dialog.addEventListener("modal:open", () => {
    const next = currentReturnTarget();
    if (google) google.href = withNext(google.href, next);
    if (logout) logout.action = withNext(logout.action, next);
  });

  // A returned popup error rode in on the URL and made the dialog open on load
  // (generic). Drop the param from the visible URL so a reload or a shared copy
  // of this address is clean and won't reopen the error.
  if (
    errorParam &&
    dialog.hasAttribute("data-modal-open-on-load") &&
    new URL(location.href).searchParams.has(errorParam)
  ) {
    const clean = new URL(location.href);
    clean.searchParams.delete(errorParam);
    history.replaceState(history.state, "", `${clean.pathname}${clean.search}${clean.hash}`);
  }
}
