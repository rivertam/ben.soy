/* The /diary service worker (registered by diary.js with scope "/diary" —
 * public pages are never controlled). Two jobs:
 *
 * 1. Offline reads: keep a device-local copy of the last good GET /diary and
 *    serve it when the network can't; hashed /_topcoat/assets/ ride a
 *    cache-first cache (immutable by contract).
 * 2. The write queue: flush pending IndexedDB entries to the JSON endpoint.
 *    Flushing lives HERE and only here — pages enqueue and kick (Background
 *    Sync, or a message where sync is missing) — so the replay policy has
 *    exactly one implementation. The server dedupes replays (same second,
 *    same body); that is the real idempotency guarantee, the Web Lock below
 *    only trims wasted duplicate work.
 */

"use strict";

const PAGE_CACHE = "diary-page-v1";
const ASSET_CACHE = "diary-assets-v1";
const LIVE_CACHES = [PAGE_CACHE, ASSET_CACHE];
const DIARY_PATH = "/diary";
const API_PATH = "/api/diary/entries";
const SYNC_TAG = "diary-flush";
const FLUSH_LOCK = "diary-flush";

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(activate());
});

async function activate() {
  await self.clients.claim();
  for (const name of await caches.keys()) {
    if (name.startsWith("diary-") && !LIVE_CACHES.includes(name)) {
      await caches.delete(name);
    }
  }
}

self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (event.request.mode === "navigate") {
    event.respondWith(navigationResponse(event.request, url));
    return;
  }
  if (
    event.request.method === "GET" &&
    url.origin === self.location.origin &&
    url.pathname.startsWith("/_topcoat/assets/")
  ) {
    event.respondWith(assetResponse(event.request));
  }
  // Everything else — the API POSTs, cross-origin, the rest of the site —
  // passes through with no respondWith at all.
});

/* Network-first. Only a real 200 from this origin for exactly /diary with no
 * query is cached: navigations arrive redirect-mode "manual", so the
 * signed-out 303 surfaces as an opaqueredirect (status 0, type
 * "opaqueredirect"), and caching one would poison the offline copy. Offline,
 * the cached /diary answers ANY in-scope navigation — page 1 carries full
 * entry bodies, so it is the useful fallback for permalinks too. */
async function navigationResponse(request, url) {
  try {
    const response = await fetch(request);
    if (
      response.ok &&
      response.type === "basic" &&
      url.pathname === DIARY_PATH &&
      url.search === ""
    ) {
      const cache = await caches.open(PAGE_CACHE);
      await cache.put(DIARY_PATH, response.clone());
    }
    return response;
  } catch (error) {
    const cache = await caches.open(PAGE_CACHE);
    const cached = await cache.match(DIARY_PATH);
    return cached || offlineStub();
  }
}

function offlineStub() {
  const html = `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Diary — offline</title></head>
<body style="font-family: system-ui, sans-serif; margin: 4rem auto; max-width: 32rem; padding: 0 1rem; color: #1d2126; background: #f4f5f7">
<h1 style="font-size: 1.2rem">Offline</h1>
<p>The diary needs one online visit before it can open offline.</p>
</body>
</html>`;
  return new Response(html, {
    status: 503,
    headers: { "Content-Type": "text/html; charset=utf-8" },
  });
}

async function assetResponse(request) {
  const cache = await caches.open(ASSET_CACHE);
  const hit = await cache.match(request);
  if (hit) {
    return hit;
  }
  const response = await fetch(request);
  if (response.ok && response.type === "basic") {
    await cache.put(request, response.clone());
  }
  return response;
}

self.addEventListener("sync", (event) => {
  if (event.tag === SYNC_TAG) {
    event.waitUntil(flushGuarded());
  }
});

self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "flush") {
    event.waitUntil(flushGuarded());
  }
});

function flushGuarded() {
  return navigator.locks.request(FLUSH_LOCK, { ifAvailable: true }, (lock) =>
    lock ? flush() : undefined
  );
}

/* Oldest-first; stop on anything retryable so composition order survives.
 * Throwing at the end makes this Background Sync attempt count as failed,
 * so the browser retries with backoff; auth can't be fixed by retrying
 * (and retries burn the bounded attempt budget), so that resolves quietly
 * and the page shows the sign-in banner instead. */
async function flush() {
  const entries = (await allEntries()).filter((entry) => entry.state === "pending");
  let saved = 0;
  let blocked = null;
  for (const entry of entries) {
    const outcome = await send(entry);
    if (outcome === "saved") {
      await deleteEntry(entry.qid);
      saved += 1;
    } else if (outcome === "auth" || outcome === "net") {
      blocked = outcome;
      break;
    } else {
      await markFailed(entry.qid, outcome);
    }
  }
  await broadcast(saved, blocked);
  if (blocked === "net") {
    throw new Error("diary flush interrupted; sync will retry");
  }
}

/* Returns "saved" | "auth" | "net" | a permanent-rejection reason.
 * Permanent (the entry itself can never succeed; kept for manual copy):
 * 400/409/413/415/422. 401/404 are the signed-out / wrong-account answers.
 * 403 is a same-origin/config failure, 5xx an outage, and a 200 that isn't
 * our JSON is a captive portal — never the entry's fault, all retry later. */
async function send(entry) {
  let response;
  try {
    response = await fetch(API_PATH, {
      method: "POST",
      credentials: "same-origin",
      cache: "no-store",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ written_at: entry.written_at, body: entry.body }),
    });
  } catch (error) {
    return "net";
  }
  if (response.ok) {
    try {
      const data = await response.json();
      if (data && data.status === "saved") {
        return "saved";
      }
    } catch (error) {
      // not our JSON; fall through
    }
    return "net";
  }
  if (response.status === 401 || response.status === 404) {
    return "auth";
  }
  if ([400, 409, 413, 415, 422].includes(response.status)) {
    return "rejected (HTTP " + response.status + ")";
  }
  return "net";
}

async function broadcast(saved, blocked) {
  const entries = await allEntries();
  const channel = new BroadcastChannel("diary");
  channel.postMessage({
    type: "queue-updated",
    pending: entries.filter((entry) => entry.state === "pending").length,
    failed: entries.filter((entry) => entry.state === "failed").length,
    saved,
    blocked,
  });
  channel.close();
}

/* IndexedDB helpers — mirrored in diary.js (a shared module would need a
 * second stable route just for importScripts; not worth it for ~40 lines).
 * Chrome keeps a transaction alive across microtasks, which is all these
 * single-request helpers need. */

const DB_NAME = "diary-queue";
const DB_VERSION = 1;
const STORE = "entries";

function openQueue() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE, { keyPath: "qid", autoIncrement: true });
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

function settle(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function allEntries() {
  const db = await openQueue();
  try {
    return await settle(db.transaction(STORE).objectStore(STORE).getAll());
  } finally {
    db.close();
  }
}

async function deleteEntry(qid) {
  const db = await openQueue();
  try {
    await settle(db.transaction(STORE, "readwrite").objectStore(STORE).delete(qid));
  } finally {
    db.close();
  }
}

async function markFailed(qid, reason) {
  const db = await openQueue();
  try {
    const store = db.transaction(STORE, "readwrite").objectStore(STORE);
    const entry = await settle(store.get(qid));
    if (entry && entry.state === "pending") {
      entry.state = "failed";
      entry.reason = reason;
      await settle(store.put(entry));
    }
  } finally {
    db.close();
  }
}
