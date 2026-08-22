if ("serviceWorker" in navigator) {
  navigator.serviceWorker
    .register("/fitness/sw.js", { scope: "/fitness" })
    .catch(() => {});
}

const installRegion = document.querySelector("[data-fitness-install]");
const installButton = document.querySelector(
  "[data-fitness-install-button]",
);
const installStatus = document.querySelector(
  "[data-fitness-install-status]",
);

const isAndroid = /Android/i.test(navigator.userAgent);
const isStandalone = window.matchMedia("(display-mode: standalone)").matches;
let installPrompt = null;

if (
  (location.pathname === "/fitness" || location.pathname === "/") &&
  isAndroid &&
  !isStandalone &&
  installRegion instanceof HTMLElement &&
  installButton instanceof HTMLButtonElement &&
  installStatus instanceof HTMLElement
) {
  window.addEventListener("beforeinstallprompt", (event) => {
    event.preventDefault();
    installPrompt = event;
    installRegion.hidden = false;
    installButton.hidden = false;
    installButton.disabled = false;
    installStatus.textContent = "Ready to install.";
  });

  installButton.addEventListener("click", async () => {
    if (!installPrompt) return;

    const prompt = installPrompt;
    installPrompt = null;
    installButton.disabled = true;
    installStatus.textContent = "Opening Android's install prompt…";

    try {
      const promptResult = await prompt.prompt();
      const choice = promptResult?.outcome
        ? promptResult
        : await prompt.userChoice;
      installButton.hidden = true;
      installStatus.textContent =
        choice.outcome === "accepted"
          ? "Install accepted. Wait for Android to finish, then confirm Fitness appears in Settings → Apps before returning to Garmin."
          : "Installation was dismissed. Reload this page when you want to try again.";
    } catch {
      installButton.hidden = true;
      installStatus.textContent =
        "The browser could not open the install prompt. Reload this page or use its menu's Install app action; a plain home-screen shortcut cannot receive Garmin shares.";
    }
  });

  window.addEventListener("appinstalled", () => {
    installPrompt = null;
    installRegion.hidden = false;
    installButton.hidden = true;
    installStatus.textContent =
      "Android accepted the Fitness install. It may take a few seconds to finish; wait until it appears in Settings → Apps, then reopen Garmin's share sheet.";
  });
}
