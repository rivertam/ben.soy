// /diary page glue for the Rust offline queue (docs/diary-sync.md).
//
// The device-local store is the source of truth now: every entry — pending,
// failed, or delivered — is a row under its permalink id, and ids are
// predicted at enqueue with the same probe the server runs. So this file no
// longer reconstructs transcript state. It clones the server-shipped
// <template id="diary-bubble"> for rows the server HTML doesn't show, keyed
// by data-id, and re-reads the store snapshot whenever a flush report
// arrives. "Does the DOM already show this id" is the entire reconciliation
// rule; the old five-bucket painter, provisional map, and hand-built DOM
// articles died with the delete-on-save queue.

"use strict";

const SW_URL = "/sw.js";
const SCOPE = "/diary";
const ASSET_CACHE = "diary-assets-v1";
const SYNC_TAG = "diary-flush";
const STORE_LOCK = "diary-store";
const SYNC_LOADER = "/diary-sync.js";

// Memoized wasm instantiation; nulled on rejection so the next call retries.
let wasmReady = null;
// The last flush report's blocked state: null, "auth", or "net". Decides
// whether pending bubbles wear the queued styling and "will sync" label.
let lastBlocked = null;

init();

function init() {
  if (!("serviceWorker" in navigator)) {
    return;
  }
  try {
    navigator.serviceWorker.register(SW_URL, { scope: SCOPE });
  } catch (error) {
    return;
  }
  if (navigator.storage && navigator.storage.persist) {
    navigator.storage.persist().catch(() => {});
  }
  const channel = new BroadcastChannel("diary");
  channel.onmessage = (event) => {
    if (event.data && event.data.type === "queue-updated") {
      onReport(event.data);
    }
  };
  // Coming back to the app is the moment queued entries can move.
  window.addEventListener("online", refresh);
  window.addEventListener("pageshow", refresh);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      refresh();
    }
  });
  hookForm();
  hookDiscards();
  positionTranscript();
  renderFromStore();
  kick();
  primeCaches();
}

/* ------------------------------------------------------------- wasm ---- */

function ensureWasm() {
  if (wasmReady) {
    return wasmReady;
  }
  wasmReady = (async () => {
    await loadScript(SYNC_LOADER);
    await loadScript(self.DIARY_SYNC.glue);
    await wasm_bindgen({ module_or_path: self.DIARY_SYNC.wasm });
    return wasm_bindgen;
  })();
  wasmReady.catch(() => {
    wasmReady = null;
  });
  return wasmReady;
}

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const tag = document.createElement("script");
    tag.src = src;
    tag.onload = resolve;
    tag.onerror = () => reject(new Error("failed to load " + src));
    document.head.appendChild(tag);
  });
}

/* The page and service worker share this lock across an epoch check and its
 * local-store operation. Therefore a newly installed worker cannot migrate
 * between an old page checking the ledger and interpreting a row. Browsers
 * without Web Locks retain the plain-form online fallback instead of opening
 * an unfenced local store. */
function withStoreLock(operation) {
  if (!navigator.locks) {
    return Promise.reject(new Error("Web Locks unavailable"));
  }
  return navigator.locks.request(STORE_LOCK, operation);
}

/* ---------------------------------------------------------- compose ---- */

function hookForm() {
  const form = document.getElementById("diary-compose");
  const box = document.getElementById("diary-body");
  if (!form || !box) {
    return; // permalink pages: register + kick only
  }
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    save(form, box);
  });
  // Enter sends on desktop only — (hover: hover) and (pointer: fine) keeps
  // touch keyboards' Enter as a newline — and never mid-IME composition.
  const desktop = matchMedia("(hover: hover) and (pointer: fine)");
  box.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) {
      return;
    }
    if (!desktop.matches) {
      return;
    }
    event.preventDefault();
    save(form, box);
  });
  if (desktop.matches && document.activeElement === document.body) {
    box.focus();
  }
}

/* The synchronous half: the bubble is in the DOM before ANY await, so the
 * message never blinks out of existence while wasm instantiates. */
function save(form, box) {
  const raw = box.value;
  if (!raw.trim()) {
    return;
  }
  const draft = {
    id: null,
    written_at: Math.floor(Date.now() / 1000),
    body: raw.replace(/\r\n?/g, "\n").trim(),
    state: "draft",
    reason: null,
  };
  const bubble = appendBubble(draft, true);
  box.value = "";
  box.focus();
  persist(form, box, raw, draft, bubble);
}

async function persist(form, box, raw, draft, bubble) {
  try {
    const wasm = await ensureWasm();
    const entry = JSON.parse(
      await withStoreLock(() => wasm.diary_enqueue(
        JSON.stringify({
          schema_epoch: wasm.diary_schema_epoch(),
          entry: { written_at: draft.written_at, body: raw },
          enqueued_at_ms: Date.now(),
        }),
      )),
    );
    const existing = byId(entry.id);
    if (existing && existing !== bubble) {
      // A same-second twin (double-tap): the store returned the original
      // row, whose bubble is already on the page.
      if (bubble) {
        bubble.remove();
      }
      hideQueueIfEmpty();
    } else {
      applyEntry(bubble, entry);
    }
    kick();
  } catch (error) {
    // The Rust store refused (no wasm build served, private-mode IndexedDB,
    // exhausted probes). Put the text back and fall back to the plain form
    // POST — but never while offline, where submitting would only lose it.
    if (bubble) {
      bubble.remove();
    }
    hideQueueIfEmpty();
    box.value = box.value ? raw + "\n\n" + box.value : raw;
    if (navigator.onLine === false) {
      return;
    }
    form.submit();
  }
}

/* ---------------------------------------------------------- bubbles ---- */

function byId(id) {
  return document.querySelector(
    '.diary-message[data-id="' + CSS.escape(id) + '"]',
  );
}

function appendBubble(entry, forceScroll) {
  const queue = document.getElementById("diary-queue");
  const template = document.getElementById("diary-bubble");
  if (!queue || !template) {
    return null;
  }
  const bubble = template.content.firstElementChild.cloneNode(true);
  applyEntry(bubble, entry);
  const transcript = document.getElementById("diary-transcript");
  const pin =
    forceScroll ||
    (transcript &&
      transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight <
        48);
  queue.appendChild(bubble);
  queue.hidden = false;
  const empty = document.getElementById("diary-empty");
  if (empty) {
    empty.hidden = true;
  }
  if (pin && transcript) {
    transcript.scrollTop = transcript.scrollHeight;
  }
  return bubble;
}

/* One entry -> one bubble, idempotently: every state renders by toggling
 * the template's parts, so a bubble moves draft -> pending -> synced or
 * -> failed without being rebuilt. */
function applyEntry(bubble, entry) {
  if (!bubble) {
    return;
  }
  const body = bubble.querySelector(".diary-body");
  if (body) {
    body.textContent = entry.body;
  }
  applyEntryState(bubble, entry);
}

/* Identity and sync-state presentation are one transition. Business
 * presentation stays in applyEntry; a delivery acknowledgement can carry
 * only the SavedRef and still unlock every state-dependent control. */
function applyEntryState(bubble, entry) {
  if (entry.id) {
    bubble.dataset.id = entry.id;
  }
  bubble.dataset.state = entry.state;
  const note = bubble.querySelector(".diary-note");
  const link = bubble.querySelector("a");
  const discard = bubble.querySelector(".diary-discard");
  const queuedLook =
    entry.state === "failed" || (entry.state === "pending" && lastBlocked);
  bubble.classList.toggle("diary-message-queued", Boolean(queuedLook));
  if (entry.state === "synced") {
    link.hidden = false;
    link.href = SCOPE + "/" + encodeURIComponent(entry.id);
    link.textContent = stampOf(entry);
    note.hidden = true;
    note.textContent = "";
    discard.hidden = true;
  } else {
    link.hidden = true;
    note.hidden = false;
    note.textContent = stampOf(entry) + stateLabel(entry);
    discard.hidden = entry.state !== "failed";
  }
}

function stateLabel(entry) {
  if (entry.state === "failed") {
    return " · failed — " + (entry.reason || "rejected");
  }
  if (entry.state === "pending" && lastBlocked) {
    return " · queued — will sync";
  }
  return "";
}

/* Prefer the id's embedded Eastern wall clock (the permalink's truth);
 * fall back to the device clock for drafts and synthetic keys. */
function stampOf(entry) {
  const id = entry.id || "";
  const match =
    /^(\d{4})-(\d{2})-(\d{2})T(\d{2})-(\d{2})-\d{2}-\d{2}-\d{2}$/.exec(id);
  if (match) {
    const hour = Number(match[4]);
    if (hour <= 23) {
      const date = new Date(
        Number(match[1]),
        Number(match[2]) - 1,
        Number(match[3]),
      ).toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
        year: "numeric",
      });
      const clock = hour % 12 === 0 ? 12 : hour % 12;
      const suffix = hour < 12 ? "AM" : "PM";
      return date + " · " + clock + ":" + match[5] + " " + suffix;
    }
  }
  return new Date(entry.written_at * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function hideQueueIfEmpty() {
  const queue = document.getElementById("diary-queue");
  if (queue && queue.childElementCount === 0) {
    queue.hidden = true;
  }
}

/* ------------------------------------------------------ reconciling ---- */

/* Re-read the store and make the DOM agree: add missing pending/failed
 * bubbles, refresh states and labels, drop bubbles for rows discarded
 * elsewhere. Server-rendered articles always win a duplicate id (they sit
 * earlier in the document, so byId prefers them). */
async function renderFromStore() {
  let entries;
  try {
    const wasm = await ensureWasm();
    entries = JSON.parse(await withStoreLock(() => wasm.diary_snapshot()));
  } catch (error) {
    return; // no store, no bubbles — the no-JS diary
  }
  const known = new Set();
  for (const entry of entries) {
    known.add(entry.id);
    const existing = byId(entry.id);
    if (!existing) {
      appendBubble(entry, false);
    } else if (existing.dataset.state !== "synced") {
      applyEntry(existing, entry);
    }
  }
  const queue = document.getElementById("diary-queue");
  if (!queue) {
    return;
  }
  for (const bubble of Array.from(queue.children)) {
    const id = bubble.dataset.id;
    if (!id) {
      continue; // a draft still waiting on its enqueue
    }
    const duplicate = byId(id) !== bubble;
    const gone = bubble.dataset.state !== "synced" && !known.has(id);
    if (duplicate || gone) {
      bubble.remove();
    }
  }
  hideQueueIfEmpty();
}

/* A flush report: apply the delivered identities (the rare server bump
 * rewrites one bubble's id and permalink), then reconcile the rest from the
 * store — one path for labels, prunes, and mid-flush page opens. */
function onReport(report) {
  lastBlocked = report.blocked;
  for (const ref of report.saved_refs || report.saved_entries || []) {
    const bubble = byId(ref.qid);
    if (!bubble || bubble.dataset.state === "synced") {
      continue;
    }
    applyEntryState(bubble, {
      id: ref.id,
      written_at: ref.written_at,
      state: "synced",
      reason: null,
    });
  }
  renderFromStore();
}

/* ---------------------------------------------------------- machinery ---- */

function refresh() {
  renderFromStore();
  kick();
}

/* Ask the worker to flush: Background Sync where available (it retries with
 * backoff after we're gone), plus an immediate message either way. */
async function kick() {
  if (!("serviceWorker" in navigator)) {
    return;
  }
  try {
    const registration = await navigator.serviceWorker.ready;
    if ("sync" in registration) {
      try {
        await registration.sync.register(SYNC_TAG);
      } catch (error) {
        // Background Sync denied; the message below still flushes now.
      }
    }
    if (registration.active) {
      registration.active.postMessage({ type: "flush" });
    }
  } catch (error) {
    // No controller yet; the worker flushes on activate.
  }
}

/* Pin the transcript to its newest message on load, after layout settles. */
function positionTranscript() {
  const transcript = document.getElementById("diary-transcript");
  if (!transcript) {
    return;
  }
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      transcript.scrollTop = transcript.scrollHeight;
    });
  });
}

function hookDiscards() {
  document.addEventListener("click", async (event) => {
    const button = event.target.closest(".diary-discard");
    if (!button) {
      return;
    }
    const bubble = button.closest(".diary-message");
    const id = bubble && bubble.dataset.id;
    if (!id) {
      return;
    }
    try {
      const wasm = await ensureWasm();
      await withStoreLock(() => wasm.diary_discard(id));
      bubble.remove();
      hideQueueIfEmpty();
    } catch (error) {
      // Store refused; the bubble stays and the tap can be retried.
    }
  });
}

/* ------------------------------------------------------------ caching ---- */

/* First-visit priming: the first load is uncontrolled, so without this an
 * install-then-airplane-mode launch would land on the stub. Everything here
 * rides the HTTP cache (the assets are immutable), so it is nearly free;
 * failures are fine — the worker primes its own set on activate. There is
 * no page copy to prime anymore: offline reads render from the mirror. */
async function primeCaches() {
  if (!("caches" in window)) {
    return;
  }
  try {
    const assets = await caches.open(ASSET_CACHE);
    const urls = new Set();
    for (const node of document.querySelectorAll(
      "link[href^='/_topcoat/assets/'], script[src^='/_topcoat/assets/']",
    )) {
      urls.add(node.getAttribute("href") || node.getAttribute("src"));
    }
    for (const url of urls) {
      if (!(await assets.match(url))) {
        await assets.add(url);
      }
    }
    // The sync pair follows the worker's rule, not cache.add()'s: store the
    // versioned bytes only when the server marked them immutable, so a
    // deploy-race answer under a stale ?v (served no-cache) can never stick
    // to the wrong key. The loader is mutable by design and stored as-is.
    if (self.DIARY_SYNC) {
      for (const url of [SYNC_LOADER, self.DIARY_SYNC.glue, self.DIARY_SYNC.wasm]) {
        if (await assets.match(url)) {
          continue;
        }
        const response = await fetch(url, { credentials: "same-origin" });
        const control = response.headers.get("Cache-Control") || "";
        if (
          response.ok &&
          response.type === "basic" &&
          (url === SYNC_LOADER || control.includes("immutable"))
        ) {
          await assets.put(url, response);
        }
      }
    }
  } catch (error) {
    // best-effort
  }
}
