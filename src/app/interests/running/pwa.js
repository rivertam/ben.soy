if ("serviceWorker" in navigator) {
  navigator.serviceWorker
    .register("/fitness/sw.js", { scope: "/fitness" })
    .catch(() => {});
}
