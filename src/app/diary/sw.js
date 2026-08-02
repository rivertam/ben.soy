/* The /diary service worker (registered by diary.js with scope "/diary" —
 * public pages are never controlled). Two jobs:
 *
 * 1. Offline reads: keep a device-local copy of the last good GET /diary and
 *    serve it when the network can't; hashed /_topcoat/assets/ and the
 *    versioned wasm pair ride cache-first caches (immutable by contract).
 * 2. The write queue: flush pending entries to the JSON endpoint. The queue
 *    itself is Rust — crates/diary-core compiled to wasm, stored in a local
 *    SurrealDB over IndexedDB (docs/diary-sync.md) — and this file is only
 *    the browser glue: import the module, hold the Web Lock, migrate the
 *    pre-wasm IndexedDB queue, broadcast the report, and throw on retryable
 *    trouble so Background Sync retries with backoff. Flushing lives HERE
 *    and only here — pages enqueue and kick. The server dedupes replays
 *    (same second, same body); that is the real idempotency guarantee, the
 *    lock only trims wasted duplicate work.
 */

"use strict";

const PAGE_CACHE = "diary-page-v1";
const ASSET_CACHE = "diary-assets-v1";
const LIVE_CACHES = [PAGE_CACHE, ASSET_CACHE];
const DIARY_PATH = "/diary";
const API_PATH = "/api/diary/entries";
const SYNC_TAG = "diary-flush";
const FLUSH_LOCK = "diary-flush";
const SYNC_LOADER = "/diary-sync.js";
const SYNC_PREFIX = "/diary-sync";

/* Both imports happen at evaluation time — Chrome refuses importScripts of
 * new URLs once install completes — and deliberately WITHOUT a try/catch:
 * if the loader or the glue it pins can't be fetched, evaluation fails,
 * this worker version never installs, and the previous working version
 * keeps running. (In a dev checkout before `just wasm` no version installs
 * at all — the diary is then online-only, exactly as if it had no worker.)
 * The loader is served no-cache, so its bytes changing on a deploy is also
 * what tells the browser a new worker version exists. */
importScripts(SYNC_LOADER);
importScripts(self.DIARY_SYNC.glue);

let wasmReady = null;

/* Instantiating the multi-megabyte module is deferred to the first flush
 * and reused for the worker's lifetime; a failure resets so the next flush
 * retries. The Cache Storage copy (primed by diary.js) is preferred so an
 * offline wake with a cold HTTP cache can still flush once back online. */
function ensureWasm() {
  if (!wasmReady) {
    wasmReady = caches
      .match(self.DIARY_SYNC.wasm)
      .then((cached) => wasm_bindgen({ module_or_path: cached || self.DIARY_SYNC.wasm }))
      .catch((error) => {
        wasmReady = null;
        throw error;
      });
  }
  return wasmReady;
}

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
  await dropStaleSyncAssets();
}

/* Old deploys' versioned pairs are dead weight (~4 MB each); keep only the
 * pair this worker imports, plus the loader. */
async function dropStaleSyncAssets() {
  const keep = [SYNC_LOADER, self.DIARY_SYNC.glue, self.DIARY_SYNC.wasm].map(
    (url) => new URL(url, self.location.origin).href
  );
  const cache = await caches.open(ASSET_CACHE);
  for (const request of await cache.keys()) {
    if (
      new URL(request.url).pathname.startsWith(SYNC_PREFIX) &&
      !keep.includes(request.url)
    ) {
      await cache.delete(request);
    }
  }
}

self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (event.request.mode === "navigate") {
    event.respondWith(navigationResponse(event.request, url));
    return;
  }
  if (event.request.method !== "GET" || url.origin !== self.location.origin) {
    return;
  }
  if (url.pathname === SYNC_LOADER) {
    event.respondWith(loaderResponse(event.request));
    return;
  }
  if (url.pathname.startsWith("/_topcoat/assets/") || url.pathname.startsWith(SYNC_PREFIX)) {
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

/* Cache-first: hashed /_topcoat/assets/ and the ?v-keyed glue/wasm are
 * immutable by contract, so a hit is always right. Sync-pair responses are
 * stored only when the server marked them immutable — a deploy-race answer
 * under a stale ?v arrives no-cache and must not stick. */
async function assetResponse(request) {
  const cache = await caches.open(ASSET_CACHE);
  const hit = await cache.match(request);
  if (hit) {
    return hit;
  }
  const response = await fetch(request);
  if (response.ok && response.type === "basic" && cacheableAsset(request, response)) {
    await cache.put(request, response.clone());
  }
  return response;
}

function cacheableAsset(request, response) {
  if (!new URL(request.url).pathname.startsWith(SYNC_PREFIX)) {
    return true;
  }
  const control = response.headers.get("Cache-Control") || "";
  return control.includes("immutable");
}

/* The loader is the one mutable sync URL (it names the current pair), so it
 * is network-first; offline falls back to the cached copy, whose pair sits
 * cached beside it. */
async function loaderResponse(request) {
  const cache = await caches.open(ASSET_CACHE);
  try {
    const response = await fetch(request);
    if (response.ok && response.type === "basic") {
      await cache.put(request, response.clone());
    }
    return response;
  } catch (error) {
    const cached = await cache.match(request);
    if (cached) {
      return cached;
    }
    throw error;
  }
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

/* The queue policy — oldest-first, stop on retryable trouble, mark
 * permanent rejections failed — lives in Rust now (diary-core::outbox);
 * so does the POST itself. This wrapper migrates, delegates, reports.
 * Throwing on "net" makes this Background Sync attempt count as failed, so
 * the browser retries with backoff; auth can't be fixed by retrying (and
 * retries burn the bounded attempt budget), so that resolves quietly and
 * the page shows the sign-in banner instead. */
async function flush() {
  await ensureWasm();
  await migrateLegacy();
  const report = JSON.parse(await wasm_bindgen.diary_flush(API_PATH));
  await broadcast(report);
  if (report.blocked === "net") {
    throw new Error("diary flush interrupted; sync will retry");
  }
}

/* One-way move of the pre-wasm queue (IndexedDB "diary-queue") into the
 * Rust store, running under the flush lock. Delete-after-import plus the
 * import's (written_at, body) dedupe make a crash at any point safe to
 * re-run; the emptied database is left behind for any straggler old
 * worker. */
async function migrateLegacy() {
  let entries;
  try {
    entries = await allLegacyEntries();
  } catch (error) {
    return; // no IndexedDB access, so no legacy queue either
  }
  if (!entries.length) {
    return;
  }
  await wasm_bindgen.diary_import(JSON.stringify(entries));
  for (const entry of entries) {
    await deleteLegacyEntry(entry.qid);
  }
}

async function broadcast(report) {
  const channel = new BroadcastChannel("diary");
  channel.postMessage({
    type: "queue-updated",
    pending: report.pending,
    failed: report.failed,
    saved: report.saved,
    blocked: report.blocked,
  });
  channel.close();
}

/* Legacy IndexedDB helpers — read and delete only, for the migration above.
 * The shapes match what diary.js used to write (db "diary-queue", store
 * "entries", keyPath "qid"). */

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

async function allLegacyEntries() {
  const db = await openQueue();
  try {
    return await settle(db.transaction(STORE).objectStore(STORE).getAll());
  } finally {
    db.close();
  }
}

async function deleteLegacyEntry(qid) {
  const db = await openQueue();
  try {
    await settle(db.transaction(STORE, "readwrite").objectStore(STORE).delete(qid));
  } finally {
    db.close();
  }
}
