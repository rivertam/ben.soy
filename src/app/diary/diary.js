/* Page-side companion to the /diary service worker (sw.js). The queue lives
 * in Rust (crates/diary-core compiled to wasm, a local SurrealDB over
 * IndexedDB — docs/diary-sync.md); this file loads that module, enqueues,
 * kicks the worker (Background Sync when available, a message otherwise),
 * and renders pending/failed messages at the live end of the transcript.
 * Online and offline saves are the same path — enqueue, then the worker
 * does all the POSTing. Without JavaScript the plain form POST to
 * /diary/write still works; if the wasm store refuses (no build served,
 * private-mode IndexedDB, disk), saves fall back to that same form POST so
 * text always has a path out.
 */

const SW_URL = "/sw.js";
const SCOPE = "/diary";
const PAGE_CACHE = "diary-page-v1";
const ASSET_CACHE = "diary-assets-v1";
const SYNC_TAG = "diary-flush";
const SYNC_LOADER = "/diary-sync.js";

let channel = null;
let submittedThisSession = false;
let lastBlocked = null;
let wasmReady = null;

init();

async function init() {
  positionTranscript();
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

/* The Rust queue module, loaded once per page: the loader names the current
 * glue/wasm pair by content hash, the glue is a classic script (the worker
 * importScripts the very same files), and instantiation resolves to the
 * wasm_bindgen namespace. Rejection means no build is being served (a dev
 * checkout before `just wasm`) or storage refused — save() then falls back
 * to the plain form POST. */
function ensureWasm() {
  if (!wasmReady) {
    wasmReady = (async () => {
      await loadScript(SYNC_LOADER);
      await loadScript(self.DIARY_SYNC.glue);
      await wasm_bindgen({ module_or_path: self.DIARY_SYNC.wasm });
      return wasm_bindgen;
    })().catch((error) => {
      wasmReady = null;
      throw error;
    });
  }
  return wasmReady;
}

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = src;
    script.onload = resolve;
    script.onerror = () => reject(new Error("failed to load " + src));
    document.head.append(script);
  });
}

/* A chat opens at the present. The server renders messages chronologically,
 * and this moves the transcript viewport to its bottom; older messages are
 * then revealed by scrolling upward. Two frames let font/layout settling
 * finish before measuring the scroll height. */
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

function refresh() {
  renderQueue();
  kick();
}

/* Re-registering the sync tag is cheap and idempotent, and Chrome retries a
 * failed sync only a few times with backoff before dropping the tag — so
 * every page open re-arms it. Registering while online fires the sync
 * immediately, which is what makes the online submit the same code path as
 * the offline one. Deliberately unconditional (the old pending-check is
 * gone): the worker is the only side that can see BOTH queues while the
 * legacy migration exists, so every open must give it a chance to look.
 * An empty flush is one wasm call and no network. */
async function kick() {
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
    const wasm = await ensureWasm();
    await wasm.diary_enqueue(Math.floor(Date.now() / 1000), textarea.value, Date.now());
  } catch (error) {
    // The wasm store refused; give the text its no-JS path to the server.
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
    window.location.assign(SCOPE);
  }
}

async function renderQueue() {
  const section = document.getElementById("diary-queue");
  if (!section) {
    return;
  }
  let entries;
  try {
    const wasm = await ensureWasm();
    entries = JSON.parse(await wasm.diary_snapshot());
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
  positionTranscript();
}

/* Queue items render with textContent only — entry text must never become
 * markup. */
function queuedArticle(entry, label, action) {
  const article = element("article", "diary-message diary-message-queued");
  const body = element("p", "leading-relaxed whitespace-pre-wrap text-ink2");
  body.textContent = entry.body;
  const meta = element("p", "mt-2 text-right font-meta text-[0.6875rem] text-muted");
  meta.textContent = stamp(entry.written_at) + " · " + label;
  article.append(body, meta);
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
    try {
      const wasm = await ensureWasm();
      await wasm.diary_discard(qid);
    } catch (error) {
      return; // the entry stays visible; discarding can be retried
    }
    renderQueue();
  });
  return button;
}

function element(tag, className) {
  // Tailwind utilities here must also appear in .rs files because its scan
  // does not read .js; diary-* component classes live in diary.css.
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
 * as /diary. The wasm pair is primed too, so the first offline open can
 * still render and enqueue. */
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
    if (self.DIARY_SYNC) {
      urls.add(SYNC_LOADER);
      urls.add(self.DIARY_SYNC.glue);
      urls.add(self.DIARY_SYNC.wasm);
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
