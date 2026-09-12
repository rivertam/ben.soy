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
  hookReplies();
  hookNow();
  initToday();
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
  const cancel = document.getElementById("diary-reply-cancel");
  if (cancel) {
    cancel.addEventListener("click", () => clearReplyTarget(box));
  }
}

function replyTarget() {
  const input = document.getElementById("diary-reply-to");
  const value = input && input.value.trim();
  return value || undefined;
}

function setReplyTarget(id, box) {
  const input = document.getElementById("diary-reply-to");
  const banner = document.getElementById("diary-replying");
  const stamp = document.getElementById("diary-replying-stamp");
  if (!input || !banner || !stamp) {
    return;
  }
  input.value = id;
  stamp.textContent = stampOf({ id: id, written_at: 0 });
  banner.hidden = false;
  if (box) {
    box.focus();
  }
}

function clearReplyTarget(box) {
  const input = document.getElementById("diary-reply-to");
  const banner = document.getElementById("diary-replying");
  const stamp = document.getElementById("diary-replying-stamp");
  if (input) {
    input.value = "";
  }
  if (stamp) {
    stamp.textContent = "";
  }
  if (banner) {
    banner.hidden = true;
  }
  if (box) {
    box.focus();
  }
}

/* The synchronous half: the bubble is in the DOM before ANY await, so the
 * message never blinks out of existence while wasm instantiates. */
async function save(form, box) {
  const raw = box.value;
  const emojiBox = document.getElementById("diary-emoji");
  const emoji = emojiBox.value.trim();
  const status = document.getElementById("diary-now-status");
  if (!raw.trim() && !emoji) {
    status.textContent = "Add an emoji, some words, or both.";
    return;
  }
  if (form.dataset.saving) return;
  form.dataset.saving = "true";
  const parent = replyTarget();
  let wasm;
  try {
    wasm = await ensureWasm();
  } catch (error) {
    delete form.dataset.saving;
    if (navigator.onLine === false) {
      status.textContent = "The device store is unavailable. Your entry is still here.";
      return;
    }
    form.submit();
    return;
  }
  try {
    const now = Date.now();
    const second = Math.floor(Date.now() / 1000);
    const content = { written_at: second, body: raw, saved_at_ms: now,
      occurred_at: second };
    if (emoji) content.emoji = emoji;
    if (parent) content.reply_to = parent;
    const entry = JSON.parse(await withStoreLock(() => wasm.diary_enqueue(JSON.stringify({
      schema_epoch: wasm.diary_schema_epoch(), entry: content, enqueued_at_ms: now,
    }))));
    if (box.value === raw) box.value = "";
    emojiBox.value = "";
    updateEmojiSelection();
    clearReplyTarget(box);
    appendBubble(entry, true);
    status.textContent = "Saved on this device.";
    await renderFromStore();
    kick();
  } catch (error) {
    status.textContent = "Couldn’t save. Use one emoji, some words, or both; your entry is still here.";
  } finally {
    delete form.dataset.saving;
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
    body.textContent = entry.edit?.deleted ? "The deletion couldn’t sync. Discard this failed change to restore the saved entry." : entry.body;
  }
  const emoji = bubble.querySelector(".diary-entry-emoji");
  if (emoji) { emoji.textContent = entry.emoji || ""; emoji.hidden = !entry.emoji; }
  bubble.dataset.emoji = entry.emoji || "";
  bubble.dataset.occurredAt = entry.occurred_at ?? entry.written_at;
  const edit = bubble.querySelector(".diary-edit-now");
  if (edit) edit.hidden = !entry.id || entry.edit?.deleted;
  const parent = entry.reply_to || "";
  bubble.dataset.replyTo = parent;
  const replyTo = bubble.querySelector(".diary-reply-to");
  const replyLink = replyTo && replyTo.querySelector("a");
  if (replyTo && replyLink) {
    if (parent) {
      replyTo.hidden = false;
      replyLink.href = SCOPE + "/" + encodeURIComponent(parent);
      replyLink.textContent = "↳ " + stampOf({ id: parent, written_at: 0 });
    } else {
      replyTo.hidden = true;
      replyLink.removeAttribute("href");
      replyLink.textContent = "";
    }
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
  const link = bubble.querySelector(".diary-permalink");
  const reply = bubble.querySelector(".diary-reply");
  const discard = bubble.querySelector(".diary-discard");
  const queuedLook =
    entry.state === "failed" || (entry.state === "pending" && lastBlocked);
  bubble.classList.toggle("diary-message-queued", Boolean(queuedLook));
  if (reply) {
    reply.hidden = entry.state !== "synced" || !entry.id;
  }
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
  if (entry.occurred_at != null) {
    return new Date(entry.occurred_at * 1000).toLocaleString("en-US", {
      timeZone: "America/New_York", month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit",
    });
  }
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
const nowRows = new Map();

async function renderFromStore() {
  let data;
  try {
    const wasm = await ensureWasm();
    data = JSON.parse(await withStoreLock(() => wasm.diary_now_data()));
  } catch (error) { return; }
  nowRows.clear();
  for (const entry of data.entries) nowRows.set(entry.id, entry);
  const recent = document.getElementById("diary-recent-emojis");
  const emojiTemplate = document.getElementById("diary-emoji-button");
  if (recent && emojiTemplate) {
    recent.replaceChildren(...data.recent.map((emoji) => {
      const button = emojiTemplate.content.firstElementChild.cloneNode(true);
      button.dataset.emoji = emoji;
      button.textContent = emoji;
      button.setAttribute("aria-label", "Select " + emoji);
      return button;
    }));
  }
  updateEmojiSelection();
  const room = document.querySelector(".diary-room");
  if (!room) return;
  const entries = data.entries.filter((entry) => !entry.edit?.deleted || entry.state === "failed");
  const page = Number(room.dataset.page) || 1;
  const sorted = [...entries].sort((a, b) =>
    (b.occurred_at ?? b.written_at) - (a.occurred_at ?? a.written_at) || b.id.localeCompare(a.id));
  const wanted = room.dataset.search ? entries.filter((entry) => entry.state !== "synced")
    : sorted.slice((page - 1) * data.page_size, page * data.page_size).reverse();
  // Until the first pull, the server HTML may contain rows not yet mirrored.
  if (!data.entries.length) return;
  const queue = document.getElementById("diary-queue");
  const history = room.querySelector(".diary-history");
  const target = history || queue;
  const keep = new Set(wanted.map((entry) => entry.id));
  for (const bubble of room.querySelectorAll(".diary-message[data-id]")) {
    if (room.dataset.search && bubble.dataset.state === "synced"
      && !nowRows.get(bubble.dataset.id)?.edit?.deleted) continue;
    if (!keep.has(bubble.dataset.id)) bubble.remove();
  }
  for (const entry of wanted) {
    const bubble = byId(entry.id) || appendBubble(entry, false);
    applyEntry(bubble, entry);
    if (bubble) target.appendChild(bubble);
  }
  hideQueueIfEmpty();
  const empty = document.getElementById("diary-empty");
  if (empty) empty.hidden = entries.length > 0;
}

function updateEmojiSelection() {
  const input = document.getElementById("diary-emoji");
  if (!input) return;
  const emoji = input.value.trim();
  const preview = document.getElementById("diary-emoji-preview");
  if (preview) preview.textContent = emoji || "☺";
  const trigger = document.querySelector("#diary-emoji-picker > summary");
  if (trigger) {
    trigger.setAttribute("aria-label", emoji ? "Change emoji " + emoji : "Choose an emoji");
    trigger.title = emoji ? "Change emoji " + emoji : "Choose an emoji";
  }
  for (const choice of document.querySelectorAll("button[data-emoji]")) {
    choice.setAttribute("aria-pressed", String(choice.dataset.emoji === emoji));
  }
}

function hookNow() {
  const picker = document.getElementById("diary-emoji-picker");
  picker?.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && event.target.id === "diary-emoji") {
      event.preventDefault();
      updateEmojiSelection();
      picker.open = false;
      document.getElementById("diary-body")?.focus();
    }
    if (event.key === "Escape" && picker.open) {
      event.preventDefault();
      picker.open = false;
      picker.querySelector("summary").focus();
    }
  });
  document.addEventListener("pointerdown", (event) => {
    if (picker?.open && !picker.contains(event.target)) picker.open = false;
  });
  document.getElementById("diary-emoji")?.addEventListener("input", updateEmojiSelection);
  document.addEventListener("click", async (event) => {
    const button = event.target.closest("button[data-emoji]");
    if (button) {
      const input = document.getElementById("diary-emoji");
      input.value = input.value === button.dataset.emoji ? "" : button.dataset.emoji;
      updateEmojiSelection();
      if (picker) picker.open = false;
      document.getElementById("diary-body")?.focus();
    }
    const edit = event.target.closest(".diary-edit-now");
    if (!edit) return;
    const id = edit.closest(".diary-message").dataset.id;
    await renderFromStore();
    const row = nowRows.get(id);
    const form = document.getElementById("diary-edit");
    const dialog = document.getElementById("diary-edit-dialog");
    if (!row || !form || !dialog) return;
    const wasm = await ensureWasm();
    form.elements.path.value = row.id;
    form.elements.revision.value = row.revision || "";
    form.elements.body.value = row.body;
    form.elements.emoji.value = row.emoji || "";
    form.elements.at.value = wasm.diary_local_time(BigInt(row.occurred_at ?? row.written_at));
    document.getElementById("diary-edit-status").textContent = "";
    dialog.showModal();
  });
  document.getElementById("diary-edit-cancel")?.addEventListener("click", () => {
    document.getElementById("diary-edit-dialog").close();
  });
  const form = document.getElementById("diary-edit");
  if (!form) return;
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const deleted = event.submitter?.name === "delete";
    if (deleted && !window.confirm("Delete this Now entry?")) return;
    if (form.dataset.saving) return;
    form.dataset.saving = "true";
    const status = document.getElementById("diary-edit-status");
    try {
      const wasm = await ensureWasm();
      const command = { id: form.elements.path.value, body: form.elements.body.value,
        emoji: form.elements.emoji.value.trim() || null,
        occurred_at: Number(wasm.diary_parse_time(form.elements.at.value)),
        deleted, now_ms: Date.now(), expected_revision: form.elements.revision.value || null };
      const row = JSON.parse(await withStoreLock(() => wasm.diary_revise(JSON.stringify(command))));
      form.elements.revision.value = row.revision || "";
      status.textContent = deleted ? "Deleted on this device." : "Saved on this device.";
      document.getElementById("diary-edit-dialog")?.close();
      if (deleted) {
        byId(row.id)?.remove();
        if (!document.getElementById("diary-edit-dialog")) {
          for (const input of form.elements) input.disabled = true;
        }
      }
      await renderFromStore();
      kick();
    } catch (error) {
      status.textContent = error.message || "Couldn’t save. Your entry is still here.";
    } finally { delete form.dataset.saving; }
  });
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
  refreshCues().catch(() => {});
}

/* ---------------------------------------------------------- machinery ---- */

function refresh() {
  renderFromStore();
  refreshToday();
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

function hookReplies() {
  document.addEventListener("click", (event) => {
    const button = event.target.closest(".diary-reply");
    if (!button) {
      return;
    }
    const bubble = button.closest(".diary-message");
    // A reply can point only at a server-confirmed permalink. Keep this
    // guard even though Rust markup and applyEntryState hide the control:
    // programmatic clicks must not bypass the state rule.
    if (!bubble || bubble.dataset.state !== "synced") {
      return;
    }
    const id = bubble.dataset.id;
    if (!id) {
      return;
    }
    const box = document.getElementById("diary-body");
    setReplyTarget(id, box);
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

/* Today is a Rust session with a DOM adapter. Browser facts/events go in;
 * view state and effects come out. JS does not own drafts, budgets, save
 * commands, connection policy, acknowledgement checks, or closure rules. */
let todayEditor = null;

async function initToday() {
  const root = document.getElementById("diary-today");
  if (!root) return;
  const status = document.getElementById("diary-today-status");
  try {
    const wasm = await ensureWasm();
    todayEditor = {
      root, wasm, status, box: document.getElementById("diary-reflection"),
      session: new wasm.DiaryToday(root.dataset.day, root.dataset.reflection || "null"),
      view: null, requests: new Set(), releaseLock: null,
    };
    document.getElementById("diary-start").addEventListener("click", () => dispatchToday({ type: "start" }));
    document.getElementById("diary-finish").addEventListener("click", () => dispatchToday({ type: "finish" }));
    const box = todayEditor.box;
    const interact = () => dispatchToday({ type: "interact" });
    const pause = () => dispatchToday({ type: "pause" });
    for (const name of ["keydown", "pointerdown", "wheel", "compositionstart", "compositionupdate"]) {
      box.addEventListener(name, interact, { passive: true });
    }
    box.addEventListener("pointermove", (event) => { if (event.buttons) interact(); });
    box.addEventListener("beforeinput", (event) => {
      interact();
      if (todayEditor.view.read_only) event.preventDefault();
    });
    box.addEventListener("input", () => dispatchToday({ type: "input", body: box.value }));
    box.addEventListener("blur", pause);
    window.addEventListener("blur", pause);
    window.addEventListener("pagehide", pause);
    window.addEventListener("offline", () => dispatchToday({ type: "offline" }));
    window.addEventListener("beforeunload", (event) => {
      pause();
      if (todayEditor.view.warn_before_leave) {
        event.preventDefault();
        event.returnValue = "";
      }
    });
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState !== "visible") pause();
    });
    setInterval(() => dispatchToday({ type: "tick" }), 100);
    refreshToday();
    refreshCues().catch(() => {});
  } catch (error) {
    status.textContent = "Connect and reload to start Today. Your saved reflection stays available on the server.";
  }
}

function refreshToday() {
  return dispatchToday({ type: "refresh" });
}

function dispatchToday(event) {
  const ed = todayEditor;
  if (!ed) return;
  const output = JSON.parse(ed.session.dispatch(JSON.stringify({ event, environment: {
    now_ms: performance.now(), wall_ms: Date.now(), visible: document.visibilityState === "visible",
    focused: document.hasFocus(), online: navigator.onLine !== false,
  } })));
  ed.view = output.view;
  paintToday(output);
  for (const effect of output.effects) {
    switch (effect.type) {
      case "request": {
        const request = todayRequest(effect).then((snapshot) => dispatchToday({ type: "response", id: effect.id, snapshot }))
          .catch((error) => dispatchToday({ type: "failed", id: effect.id, reason: error.message }))
          .finally(() => ed.requests.delete(request));
        ed.requests.add(request);
        break;
      }
      case "acquire_lock":
        navigator.locks.request("diary-today-editor", { ifAvailable: true }, async (lock) => {
          if (!lock) return void dispatchToday({ type: "lock", acquired: false });
          const held = new Promise((resolve) => { ed.releaseLock = resolve; });
          try {
            dispatchToday({ type: "lock", acquired: true });
            await held;
          } finally { ed.releaseLock = null; }
        }).catch(() => dispatchToday({ type: "lock", acquired: false }));
        break;
      case "release_lock": ed.releaseLock?.(); break;
      case "focus_editor": ed.box.focus(); break;
    }
  }
}

function todayRequest(effect) {
  return new Promise((resolve, reject) => {
    const channel = new MessageChannel();
    const timeout = setTimeout(() => {
      channel.port1.close(); reject(new Error("offline"));
    }, effect.timeout_ms);
    const fail = (error) => { clearTimeout(timeout); channel.port1.close(); reject(error); };
    channel.port1.onmessage = (event) => {
      clearTimeout(timeout);
      channel.port1.close();
      if (event.data.ok) resolve(event.data.snapshot);
      else reject(new Error(event.data.error || "offline"));
    };
    navigator.serviceWorker.ready.then((registration) => {
      if (!registration.active) return fail(new Error("offline"));
      const command = effect.command ? JSON.stringify(effect.command) : "";
      registration.active.postMessage({ type: "today-request", command, day: effect.day }, [channel.port2]);
    }).catch(fail);
  });
}

function paintToday(output) {
  const ed = todayEditor;
  const view = output.view;
  if (output.replace_body !== null) ed.box.value = output.replace_body;
  ed.box.readOnly = view.read_only;
  ed.status.textContent = view.status;
  ed.root.dataset.closed = String(view.closed);
  document.getElementById("diary-time").textContent = view.time;
  document.getElementById("diary-clock-state").textContent = view.phase;
  document.getElementById("diary-writing").hidden = !view.editor_visible;
  document.getElementById("diary-prompts").hidden = !view.prompts_visible;
  const finish = document.getElementById("diary-finish");
  finish.hidden = !view.finish_visible;
  finish.disabled = view.finish_disabled;
  const start = document.getElementById("diary-start");
  start.hidden = !view.start_visible;
  start.disabled = view.start_disabled;
  start.textContent = view.start_label;
  start.setAttribute("aria-expanded", String(view.prompts_visible));
  const cell = ed.root.querySelector('.diary-heatmap [data-day="' + CSS.escape(ed.root.dataset.day) + '"]');
  if (cell) {
    cell.dataset.status = view.day_status;
    cell.title = ed.root.dataset.day + ": " + view.day_status;
    cell.setAttribute("aria-label", cell.title);
  }
}

async function refreshCues() {
  const ed = todayEditor;
  if (!ed) return;
  const data = JSON.parse(await withStoreLock(() => ed.wasm.diary_now_cues(ed.root.dataset.day)));
  if (!data.has_history) return;
  const target = document.getElementById("diary-cues");
  const template = document.getElementById("diary-cue-template");
  target.replaceChildren(...data.cues.map((entry) => {
    const bubble = template.content.firstElementChild.cloneNode(true);
    applyEntry(bubble, entry);
    bubble.querySelector(".diary-edit-now").hidden = true;
    bubble.querySelector(".diary-reply").hidden = true;
    return bubble;
  }));
  if (!data.cues.length) target.textContent = "No Now entries for this day.";
}
