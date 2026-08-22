/* The Fitness app is installable and shareable, not offline. Keeping this
 * worker network-free means it never shadows the site's normal server/cache
 * behavior or touches the diary's device-local data. */
self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});
