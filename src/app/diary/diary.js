/* Page-side companion to the /diary service worker (sw.js). Saves go
 * IndexedDB-first — online and offline are the same path — and the worker
 * does all the POSTing; this file enqueues, kicks (Background Sync when
 * available, a message otherwise), and renders the pending/failed queue
 * above the entry list. Without JavaScript the plain form POST to
 * /diary/write still works; if IndexedDB refuses (private mode, disk),
 * saves fall back to that same form POST so text always has a path out.
 */

const SW_URL = "/sw.js";
const SCOPE = "/diary";
const PAGE_CACHE = "diary-page-v1";
const ASSET_CACHE = "diary-assets-v1";
const SYNC_TAG = "diary-flush";

let channel = null;
let submittedThisSession = false;
let lastBlocked = null;

init();

async function init() {
  if (!("serviceWorker" in navigator)) {
    return;
  }
  try {
    await navigator.serviceWorker.register(SW_URL, { scope: SCOPE });
  } catch (error) {
    return; // no worker, no offline — the plain form still works
  }
  if (navigator.storage && navigator.storage.persist) {
    // Installed PWAs get persistence silently; it keeps the queue from
    // being evicted under storage pressure.
    navigator.storage.persist().catch(() => {});
  }
  channel = new BroadcastChannel("diary");
  channel.onmessage = (event) => {
    onQueueUpdated(event.data);
  };
  hookForm();
  await renderQueue();
  primeCaches();
  kick();
  window.addEventListener("online", refresh);
  window.addEventListener("pageshow", refresh);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      refresh();
    }
  });
}

function refresh() {
  renderQueue();
  kick();
}

/* Re-registering the sync tag is cheap and idempotent, and Chrome retries a
 * failed sync only a few times with backoff before dropping the tag — so
 * every page open re-arms it while anything is still pending. Registering
 * while online fires the sync immediately, which is what makes the online
 * submit the same code path as the offline one. */
async function kick() {
  let entries;
  try {
    entries = await allEntries();
  } catch (error) {
    return;
  }
  if (!entries.some((entry) => entry.state === "pending")) {
    return;
  }
  const registration = await navigator.serviceWorker.ready;
  if ("sync" in registration) {
    try {
      await registration.sync.register(SYNC_TAG);
    } catch (error) {
      // registration can fail; the message below still flushes
    }
  }
  // Always ALSO kick directly while a page is open: re-registering a sync
  // tag that already failed once leaves it waiting out Chrome's backoff
  // with no immediate attempt (verified locally). The flush lock and the
  // server's replay dedupe make the overlap harmless, and Background Sync
  // still covers the closed-app case.
  if (registration.active) {
    registration.active.postMessage({ type: "flush" });
  }
}

function hookForm() {
  const form = document.getElementById("diary-compose");
  const textarea = document.getElementById("diary-body");
  if (!form || !textarea) {
    return; // entry permalink pages: register + kick only
  }
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    save(form, textarea);
  });
}

async function save(form, textarea) {
  if (!textarea.value.trim()) {
    return;
  }
  const button = form.querySelector("button[type=submit]");
  if (button) {
    button.disabled = true;
  }
  try {
    await addEntry({
      written_at: Math.floor(Date.now() / 1000),
      body: textarea.value,
      state: "pending",
      reason: null,
      enqueued_at: Date.now(),
    });
  } catch (error) {
    // IndexedDB refused; give the text its no-JS path to the server.
    if (button) {
      button.disabled = false;
    }
    form.submit();
    return;
  }
  submittedThisSession = true;
  textarea.value = "";
  if (button) {
    button.disabled = false;
  }
  await renderQueue();
  kick();
}

function onQueueUpdated(message) {
  if (!message || message.type !== "queue-updated") {
    return;
  }
  lastBlocked = message.blocked;
  renderQueue();
  // Show server truth once a flush lands something saved this session; the
  // guard keeps other open tabs from reload-looping each other.
  if (message.saved > 0 && message.pending === 0 && submittedThisSession) {
    submittedThisSession = false;
    window.location.reload();
  }
}

async function renderQueue() {
  const section = document.getElementById("diary-queue");
  if (!section) {
    return;
  }
  let entries;
  try {
    entries = await allEntries();
  } catch (error) {
    return;
  }
  const pending = entries.filter((entry) => entry.state === "pending");
  const failed = entries.filter((entry) => entry.state === "failed");
  section.textContent = "";
  section.hidden = pending.length === 0 && failed.length === 0;
  if (section.hidden) {
    return;
  }
  if (lastBlocked === "auth" && pending.length > 0) {
    const banner = element("p", "max-w-prose border-l-2 border-oxide pl-3 font-meta text-sm text-ink2");
    banner.append(pending.length + " pending — ", link("/login?next=%2Fdiary", "sign in"), " to sync.");
    section.append(banner);
  }
  for (const entry of pending) {
    section.append(queuedArticle(entry, "queued — will sync", null));
  }
  for (const entry of failed) {
    section.append(
      queuedArticle(entry, "failed — " + (entry.reason || "rejected"), discardButton(entry.qid))
    );
  }
}

/* Queue items render with textContent only — entry text must never become
 * markup. */
function queuedArticle(entry, label, action) {
  const article = element("article", "border-t border-hairline py-6");
  const meta = element("p", "font-meta text-xs text-muted");
  meta.textContent = label + " · " + stamp(entry.written_at);
  const body = element("p", "mt-2 leading-relaxed whitespace-pre-wrap text-ink2");
  body.textContent = entry.body;
  article.append(meta, body);
  if (action) {
    article.append(action);
  }
  return article;
}

/* Only failed entries get an exit; pending ones are on their way. */
function discardButton(qid) {
  const button = element("button", "quiet-link mt-3 cursor-pointer font-meta text-xs");
  button.type = "button";
  button.dataset.discard = String(qid);
  button.textContent = "discard this entry";
  button.addEventListener("click", async () => {
    await deleteEntry(qid);
    renderQueue();
  });
  return button;
}

function element(tag, className) {
  // Classes here must also appear in .rs files — Tailwind's scan does not
  // read .js (see CLAUDE.md); every class below is copied from diary.rs.
  const node = document.createElement(tag);
  node.className = className;
  return node;
}

function link(href, label) {
  const anchor = document.createElement("a");
  anchor.className = "oxlink";
  anchor.href = href;
  anchor.textContent = label;
  return anchor;
}

function stamp(writtenAt) {
  return new Date(writtenAt * 1000).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/* First-visit priming: the first load is uncontrolled, so without this an
 * install-then-airplane-mode launch would land on the browser's offline
 * page. Everything here rides the HTTP cache (the assets are immutable), so
 * it is nearly free; failures are fine — the worker takes over from the
 * next load. The redirect guard keeps a login page from ever being stored
 * as /diary. */
async function primeCaches() {
  if (!("caches" in window)) {
    return;
  }
  try {
    const page = await caches.open(PAGE_CACHE);
    if (!(await page.match(SCOPE))) {
      const response = await fetch(SCOPE, { credentials: "same-origin" });
      if (response.ok && new URL(response.url).pathname === SCOPE) {
        await page.put(SCOPE, response);
      }
    }
    const assets = await caches.open(ASSET_CACHE);
    const urls = new Set();
    for (const node of document.querySelectorAll(
      "link[href^='/_topcoat/assets/'], script[src^='/_topcoat/assets/']"
    )) {
      urls.add(node.getAttribute("href") || node.getAttribute("src"));
    }
    for (const url of urls) {
      if (!(await assets.match(url))) {
        await assets.add(url);
      }
    }
  } catch (error) {
    // best-effort
  }
}

/* IndexedDB helpers — mirrored in sw.js; keep names and shapes in step
 * (the pwa.rs tests hold both files to the shared literals). */

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

async function addEntry(entry) {
  const db = await openQueue();
  try {
    await settle(db.transaction(STORE, "readwrite").objectStore(STORE).add(entry));
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
