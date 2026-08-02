/* Page-side companion to the /diary service worker (sw.js). The queue lives
 * in Rust (crates/diary-core compiled to wasm, a local SurrealDB over
 * IndexedDB — docs/diary-sync.md); this file loads that module, enqueues,
 * kicks the worker (Background Sync when available, a message otherwise),
 * and renders the live end of the transcript optimistically.
 *
 * The rendering rule: a sent message NEVER leaves the screen. It appears
 * synchronously in the submit handler (before any await), then moves
 * through three buckets — optimistic (not yet in the store), queued (in the
 * store, from the wasm snapshot), saved (on the server, from the flush
 * report's saved_entries) — and every repaint draws the union of the three,
 * reconciled by qid. Saved messages render exactly like the server's own
 * transcript markup, so the next real navigation changes nothing visually;
 * a flush deliberately triggers NO navigation at all (the old full-page
 * reload was a guaranteed disappear-and-reappear flash).
 *
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
let lastBlocked = null;
let wasmReady = null;

/* The buckets the queue section paints, plus the snapshot guard.
 * snapshotEntries may momentarily lag the store (it refreshes async); the
 * qid reconciliation below is what keeps a lagging snapshot from ever
 * double-showing or hiding a message. `provisional` covers the mid-flush
 * window: the worker deletes a saved entry from the store immediately but
 * reports the whole flush once at the end, so a pending entry that vanishes
 * from a snapshot without a report keeps rendering from here until its
 * report lands. `serverIds` is what the server-rendered transcript already
 * shows, so a late report for an entry the HTML includes (a page opened
 * mid-flush) is not drawn a second time. */
let snapshotEntries = []; // last wasm snapshot: pending + failed entries
let optimistic = []; // sent drafts not yet proven to be in the store
let sessionSaved = []; // entries this page watched reach the server
let provisional = new Map(); // qid -> entry: left the store, report pending
let savedQids = new Set(); // every qid a flush report has retired
let serverIds = new Set(); // entry ids already in the server-rendered HTML
let renderSeq = 0;

init();

async function init() {
  positionTranscript();
  collectServerIds();
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

/* The optimistic half of a send, all synchronous: the bubble is in the DOM
 * and the textarea is clear before this function returns — no await sits
 * between the click and the message being visible. The preview body mirrors
 * the Rust normalizer (CRLF→LF, trim) so the bubble doesn't shift when the
 * store's copy replaces it. A double-click's second submit sees the emptied
 * textarea and returns. */
function save(form, textarea) {
  const raw = textarea.value;
  if (!raw.trim()) {
    return;
  }
  const draft = {
    qid: null,
    written_at: Math.floor(Date.now() / 1000),
    body: raw.replace(/\r\n?/g, "\n").trim(),
  };
  optimistic.push(draft);
  textarea.value = "";
  paintQueue(true);
  persist(form, textarea, draft, raw);
}

/* The durable half: land the draft in the wasm store, then let the snapshot
 * repaint take over (the draft stays rendered until reconciliation proves
 * the store or the server holds it — see reconcileOptimistic). On refusal
 * the text goes back into the form and takes its no-JS path; anything typed
 * meanwhile rides along rather than being lost to the navigation. */
async function persist(form, textarea, draft, raw) {
  try {
    const wasm = await ensureWasm();
    draft.qid = await wasm.diary_enqueue(draft.written_at, raw, Date.now());
  } catch (error) {
    optimistic = optimistic.filter((entry) => entry !== draft);
    textarea.value = textarea.value ? raw + "\n\n" + textarea.value : raw;
    if (navigator.onLine === false) {
      // Offline AND the store refused: the form POST cannot reach the
      // server either, so submitting would only feed the text to a dead
      // navigation. Keep it visible in the box for a retry instead.
      paintQueue();
      return;
    }
    form.submit();
    return;
  }
  reconcileOptimistic();
  await renderQueue();
  kick();
}

function onQueueUpdated(message) {
  if (!message || message.type !== "queue-updated") {
    return;
  }
  lastBlocked = message.blocked;
  mergeSaved(message.saved_entries);
  reconcileOptimistic();
  // Paint immediately — queued bubbles flip to delivered with no await —
  // then true up pending/failed from the store.
  paintQueue();
  renderQueue();
}

/* The permalinks the server rendered into this page's transcript — a late
 * flush report for one of these (the page was opened mid-flush) must retire
 * bubbles, never draw the message a second time. */
function collectServerIds() {
  for (const anchor of document.querySelectorAll(".diary-history article a[href]")) {
    const path = new URL(anchor.href, location.origin).pathname;
    if (path.startsWith(SCOPE + "/")) {
      serverIds.add(decodeURIComponent(path.slice(SCOPE.length + 1)));
    }
  }
}

/* Every ref retires its qid (drafts, provisional, stale snapshots) even
 * when the message itself is not drawn again — deduped twins share a server
 * id, and a mid-flush page load already shows it in the transcript HTML. */
function mergeSaved(refs) {
  for (const ref of refs || []) {
    savedQids.add(ref.qid);
    provisional.delete(ref.qid);
    if (serverIds.has(ref.id)) {
      continue;
    }
    if (!sessionSaved.some((entry) => entry.id === ref.id)) {
      sessionSaved.push(ref);
    }
  }
}

/* Drop a draft only once the entry is provably held elsewhere — in the
 * store (snapshot) or on the server (a flush report). qid equality is
 * exact; matching on body would mean re-implementing the normalizer. Until
 * then the draft keeps rendering, which is what makes a stale snapshot
 * repaint unable to blink the message away. */
function reconcileOptimistic() {
  const held = new Set(snapshotEntries.map((entry) => entry.qid));
  optimistic = optimistic.filter(
    (draft) => draft.qid === null || !(held.has(draft.qid) || savedQids.has(draft.qid))
  );
}

/* Refresh the snapshot, then repaint. The seq guard drops a slow older
 * snapshot that resolves after a newer request started — without it a
 * pre-enqueue snapshot could overwrite a post-enqueue one. A pending entry
 * that vanished from the store without a flush report moved to the
 * provisional bucket: the only way a pending entry leaves the store is
 * being saved, so it keeps rendering until its report retires it. */
async function renderQueue() {
  if (!document.getElementById("diary-queue")) {
    return;
  }
  const seq = ++renderSeq;
  let entries;
  try {
    const wasm = await ensureWasm();
    entries = JSON.parse(await wasm.diary_snapshot());
  } catch (error) {
    return;
  }
  if (seq !== renderSeq) {
    return;
  }
  const present = new Set(entries.map((entry) => entry.qid));
  for (const entry of snapshotEntries) {
    if (entry.state === "pending" && !present.has(entry.qid) && !savedQids.has(entry.qid)) {
      provisional.set(entry.qid, entry);
    }
  }
  for (const qid of [...provisional.keys()]) {
    if (present.has(qid) || savedQids.has(qid)) {
      provisional.delete(qid);
    }
  }
  snapshotEntries = entries;
  reconcileOptimistic();
  paintQueue();
}

/* One synchronous pass from state to DOM — the browser paints only after it
 * finishes, so a repaint can never show a half-updated queue. Scroll stays
 * pinned to the bottom only when the viewport already was (or on an own
 * send); a background flush must not yank the reader away from an old
 * page. */
function paintQueue(forceScroll = false) {
  const section = document.getElementById("diary-queue");
  if (!section) {
    return;
  }
  const transcript = document.getElementById("diary-transcript");
  const pin = forceScroll || nearBottom(transcript);
  // A snapshot taken before a flush landed still lists what the flush
  // saved; the report's qids win so a message shows once, as delivered.
  const live = snapshotEntries.filter((entry) => !savedQids.has(entry.qid));
  const pending = live.filter((entry) => entry.state === "pending");
  const failed = live.filter((entry) => entry.state === "failed");
  section.textContent = "";
  section.hidden =
    sessionSaved.length === 0 &&
    pending.length === 0 &&
    failed.length === 0 &&
    optimistic.length === 0 &&
    provisional.size === 0;
  // The server's "No messages yet" placeholder contradicts any bubble; it
  // returns only when the transcript is genuinely empty again.
  const empty = document.getElementById("diary-empty");
  if (empty) {
    empty.hidden = !section.hidden;
  }
  if (!section.hidden) {
    for (const entry of sessionSaved) {
      section.append(savedArticle(entry));
    }
    const queuedCount = provisional.size + pending.length + optimistic.length;
    if (lastBlocked === "auth" && queuedCount > 0) {
      const banner = element("p", "max-w-prose border-l-2 border-oxide pl-3 font-meta text-sm text-ink2");
      banner.append(queuedCount + " pending — ", link("/login?next=%2Fdiary", "sign in"), " to sync.");
      section.append(banner);
    }
    // On-their-way entries (mid-flush provisional first — they flushed
    // first — then store-queued, then drafts) render in their FINAL form
    // while syncing is going fine: same bubble, same stamp shape, no
    // status. Painting "queued — will sync" during the happy path made
    // every send flash a dashed bubble with a longer meta line before
    // snapping to the delivered look. The queued styling appears only
    // when the last flush report actually said the queue is blocked.
    const blocked = lastBlocked === "net" || lastBlocked === "auth";
    for (const entry of [...provisional.values(), ...pending, ...optimistic]) {
      section.append(blocked ? queuedArticle(entry, "queued — will sync", null) : settlingArticle(entry));
    }
    for (const entry of failed) {
      section.append(
        queuedArticle(entry, "failed — " + (entry.reason || "rejected"), discardButton(entry.qid))
      );
    }
  }
  if (transcript && pin) {
    transcript.scrollTop = transcript.scrollHeight;
  }
}

function nearBottom(transcript) {
  if (!transcript) {
    return false;
  }
  return transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight < 48;
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

/* An entry still on its way, already in its final clothes: identical to
 * savedArticle except the stamp is plain text (no permalink exists yet),
 * colored like quiet-link so the swap to an anchor at delivery changes
 * nothing visible. The stamp comes from the local clock in the format the
 * server will use. */
function settlingArticle(entry) {
  const article = element("article", "diary-message");
  const body = element("p", "leading-relaxed whitespace-pre-wrap text-ink2");
  body.textContent = entry.body;
  const meta = element("p", "mt-2 text-right font-meta text-[0.6875rem] text-muted");
  const pendingStamp = element("span", "text-ink2");
  pendingStamp.textContent = localStamp(entry.written_at);
  meta.append(pendingStamp);
  article.append(body, meta);
  return article;
}

/* A delivered message, matching the server-rendered transcript markup
 * article-for-article (diary.rs) so the next real navigation changes
 * nothing visually — permalink included, since the flush report carries the
 * server-assigned id. */
function savedArticle(entry) {
  const article = element("article", "diary-message");
  const body = element("p", "leading-relaxed whitespace-pre-wrap text-ink2");
  body.textContent = entry.body;
  const meta = element("p", "mt-2 text-right font-meta text-[0.6875rem] text-muted");
  const permalink = element("a", "quiet-link");
  permalink.href = SCOPE + "/" + encodeURIComponent(entry.id);
  permalink.textContent = savedStamp(entry);
  meta.append(permalink);
  article.append(body, meta);
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

/* The delivered-stamp shape ("Aug 2, 2026 · 2:15 PM") from the viewer's
 * clock, for entries that don't have a server id yet. For a viewer in
 * Eastern time this matches what savedStamp will render, so delivery does
 * not visibly rewrite the line. */
function localStamp(writtenAt) {
  const date = new Date(writtenAt * 1000);
  return (
    date.toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" }) +
    " · " +
    date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
  );
}

/* The server's stamp ("Jul 27, 2026 · 2:30 PM"), read from the id's
 * embedded Eastern wall clock exactly like entry_stamp in diary.rs — Date
 * math would shift it into the viewer's zone. Unparseable ids fall back to
 * the queue stamp. */
function savedStamp(entry) {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2})-(\d{2})-\d{2}-\d{2}-\d{2}$/.exec(entry.id);
  if (!match) {
    return stamp(entry.written_at);
  }
  const [, year, month, day, hour, minute] = match;
  const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const name = months[Number(month) - 1];
  if (!name || Number(hour) > 23) {
    return stamp(entry.written_at);
  }
  const clock = Number(hour) % 12 === 0 ? 12 : Number(hour) % 12;
  const suffix = Number(hour) < 12 ? "AM" : "PM";
  return name + " " + Number(day) + ", " + year + " · " + clock + ":" + minute + " " + suffix;
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
