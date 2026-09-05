const LOADER = "/fitness-entry-wasm.js";
const DATABASE = "fitness-entry";
const DATABASE_VERSION = 1;
const STATE_STORE = "state";
const OUTBOX_STORE = "outbox";
const SYNC_TAG = "fitness-entry-flush";
const LEGACY_STORAGE_KEY = "fitness-entry-draft-v1";

// Both imports happen while the worker is evaluated. A missing or corrupt
// matched pair makes this version fail installation, preserving any previous
// working worker instead of activating a half-functional queue.
importScripts(LOADER);
if (!self.FITNESS_ENTRY_WASM?.glue || !self.FITNESS_ENTRY_WASM?.wasm) {
  throw new Error("Fitness entry wasm loader is unavailable.");
}
importScripts(self.FITNESS_ENTRY_WASM.glue);

let wasmReady = null;
let operationTail = Promise.resolve();

self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      await self.clients.claim();
      try {
        await enqueueOperation(async () => {
          const db = await openDatabase();
          await flushOutbox(db);
        });
      } catch (_error) {
        // A page startup reports the actionable error. Activation itself
        // still succeeds so a later storage/network recovery can retry.
      }
    })(),
  );
});

self.addEventListener("sync", (event) => {
  if (event.tag === SYNC_TAG) {
    event.waitUntil(
      enqueueOperation(async () => {
        const db = await openDatabase();
        const flush = await flushOutbox(db);
        if (flush.retry_pending) {
          throw new Error("Fitness entry flush interrupted; sync will retry.");
        }
      }),
    );
  }
});

self.addEventListener("message", (event) => {
  const port = event.ports?.[0];
  if (!port) return;
  const request = event.data;
  const sourceClientId = event.source?.id || null;
  const work = enqueueOperation(() => dispatch(request, sourceClientId));
  event.waitUntil(
    work.then(
      (value) => {
        port.postMessage({
          protocol: self.FITNESS_ENTRY_WASM.protocol,
          request_id: request?.request_id || "",
          ok: true,
          value,
        });
      },
      (error) => {
        port.postMessage({
          protocol: self.FITNESS_ENTRY_WASM.protocol,
          request_id: request?.request_id || "",
          ok: false,
          error: error instanceof Error ? error.message : "Fitness entry worker failed.",
        });
      },
    ),
  );
});

function enqueueOperation(operation) {
  const current = operationTail.then(operation, operation);
  operationTail = current.catch(() => {});
  return current;
}

async function wasm() {
  if (!wasmReady) {
    wasmReady = (async () => {
      if (typeof wasm_bindgen !== "function") {
        throw new Error("Fitness entry wasm glue is corrupt.");
      }
      await wasm_bindgen({ module_or_path: self.FITNESS_ENTRY_WASM.wasm });
      const required = [
        "fitness_protocol_version",
        "fitness_bootstrap",
        "fitness_transition",
        "fitness_derive",
        "fitness_finalize",
        "fitness_publication",
        "fitness_order_outbox",
        "fitness_pending_outbox",
        "fitness_classify_response",
        "fitness_apply_response",
        "fitness_restore",
        "fitness_dismiss",
      ];
      for (const name of required) {
        if (typeof wasm_bindgen[name] !== "function") {
          throw new Error(`Fitness entry wasm is missing ${name}.`);
        }
      }
      if (wasm_bindgen.fitness_protocol_version() !== self.FITNESS_ENTRY_WASM.protocol) {
        throw new Error("Fitness entry protocol versions do not match.");
      }
      return wasm_bindgen;
    })();
  }
  const attempt = wasmReady;
  try {
    return await attempt;
  } catch (error) {
    if (wasmReady === attempt) wasmReady = null;
    throw error;
  }
}

async function dispatch(request, sourceClientId) {
  if (
    !request ||
    request.protocol !== self.FITNESS_ENTRY_WASM.protocol ||
    typeof request.request_id !== "string" ||
    typeof request.method !== "string"
  ) {
    throw new Error("Fitness entry request uses an unsupported protocol.");
  }
  const module = await wasm();
  const db = await openDatabase();
  const payload = request.payload && typeof request.payload === "object" ? request.payload : {};

  switch (request.method) {
    case "bootstrap":
      return bootstrap(module, db, payload, sourceClientId);
    case "snapshot":
    case "derive":
      return snapshot(module, db, payload.context);
    case "transition":
      return transitionDraft(module, db, payload, sourceClientId);
    case "finalize":
      return finalizeDraft(module, db, payload, sourceClientId);
    case "flush":
      return flushAndSnapshot(module, db, payload.context, sourceClientId);
    case "flush_only":
      return flushOutbox(db, sourceClientId);
    case "draft_status":
      return draftStatus(db);
    case "restore":
      return restoreWorkout(module, db, payload, sourceClientId);
    case "dismiss":
      return dismissReceipt(module, db, payload, sourceClientId);
    default:
      throw new Error("Fitness entry request names an unknown operation.");
  }
}

async function bootstrap(module, db, payload, sourceClientId) {
  const stored = await readState(db);
  const output = decode(
    module.fitness_bootstrap(
      JSON.stringify({
        stored_draft: stored.draft,
        guide: payload.guide,
        now_utc: payload.now_utc,
        context: payload.context || {},
      }),
    ),
  );
  await writeState(db, output.draft, output.guide);
  const value = await composeSnapshot(db, output.draft, output.guide, output.derived, {
    restored_start_reset: output.restored_start_reset,
    legacy_storage_key: LEGACY_STORAGE_KEY,
  });
  await broadcastChange(sourceClientId);
  return value;
}

async function snapshot(module, db, context = {}) {
  const state = await requireState(db);
  const derived = decode(
    module.fitness_derive(
      JSON.stringify({ draft: state.draft, guide: state.guide, context: context || {} }),
    ),
  );
  return composeSnapshot(db, state.draft, state.guide, derived);
}

async function transitionDraft(module, db, payload, sourceClientId) {
  const state = await requireState(db);
  const output = decode(
    module.fitness_transition(
      JSON.stringify({
        draft: state.draft,
        guide: state.guide,
        action: payload.action,
        context: payload.context || {},
      }),
    ),
  );
  await writeDraft(db, output.draft);
  const value = await composeSnapshot(db, output.draft, state.guide, output.derived, {
    effect: output.effect,
    error: output.error,
  });
  await broadcastChange(sourceClientId);
  return value;
}

async function finalizeDraft(module, db, payload, sourceClientId) {
  const state = await requireState(db);
  const queueId = localId();
  const output = decode(
    module.fitness_finalize(
      JSON.stringify({
        draft: state.draft,
        guide: state.guide,
        ended_at_utc: payload.ended_at_utc,
        queue_id: queueId,
        enqueued_at_ms: Number(payload.enqueued_at_ms),
        context: payload.context || {},
      }),
    ),
  );
  if (output.error || !output.queued) {
    return composeSnapshot(db, state.draft, state.guide, output.derived, {
      error: output.error || { message: "The workout could not be finalized." },
    });
  }

  // The reset and immutable payload insertion share one IDB transaction.
  // A crash can happen before both or after both, never between them.
  await commitFinalization(db, output.draft, output.queued);
  // Arm browser-managed retry while this foreground enqueue is known to
  // exist. A sync event itself must never re-register its firing tag.
  await registerBackgroundSync();
  await broadcastChange(sourceClientId);
  const flush = await flushOutbox(db, sourceClientId);
  const derived = decode(
    module.fitness_derive(
      JSON.stringify({
        draft: output.draft,
        guide: state.guide,
        context: payload.context || {},
      }),
    ),
  );
  return composeSnapshot(db, output.draft, state.guide, derived, {
    enqueued_queue_id: queueId,
    flush,
  });
}

async function flushAndSnapshot(module, db, context = {}, sourceClientId = null) {
  const flush = await flushOutbox(db, sourceClientId);
  const value = await snapshot(module, db, context);
  value.flush = flush;
  return value;
}

async function restoreWorkout(module, db, payload, sourceClientId) {
  const state = await requireState(db);
  const queued = await readOutbox(db, payload.queue_id);
  if (!queued) throw new Error("That queued workout no longer exists.");
  const output = decode(
    module.fitness_restore(
      JSON.stringify({ draft: state.draft, queued, now_utc: payload.now_utc }),
    ),
  );
  if (output.error) {
    const value = await snapshot(module, db, payload.context);
    value.error = output.error;
    return value;
  }
  await commitRestore(db, output.draft, queued.queue_id);
  const derived = decode(
    module.fitness_derive(
      JSON.stringify({
        draft: output.draft,
        guide: state.guide,
        context: payload.context || {},
      }),
    ),
  );
  const value = await composeSnapshot(db, output.draft, state.guide, derived, {
    effect: "reset",
    restored_start_reset: output.restored_start_reset,
  });
  await broadcastChange(sourceClientId);
  return value;
}

async function dismissReceipt(module, db, payload, sourceClientId) {
  const queued = await readOutbox(db, payload.queue_id);
  if (!queued) throw new Error("That Workout Receipt no longer exists.");
  module.fitness_dismiss(JSON.stringify(queued));
  await deleteOutbox(db, queued.queue_id);
  const value = await snapshot(module, db, payload.context);
  await broadcastChange(sourceClientId);
  return value;
}

async function flushOutbox(db, sourceClientId = null) {
  const module = await wasm();
  const pending = decode(
    module.fitness_pending_outbox(JSON.stringify(await readAllOutbox(db))),
  );
  let authBlocked = false;
  let retryPending = false;
  let changed = false;

  for (const queued of pending) {
    const publication = decode(module.fitness_publication(JSON.stringify(queued)));
    let disposition;
    try {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 20_000);
      let response;
      let responseBody;
      try {
        response = await fetch(publication.path, {
          method: "POST",
          credentials: "same-origin",
          cache: "no-store",
          headers: {
            Accept: "application/json",
            "Content-Type": "application/json",
          },
          body: publication.body,
          signal: controller.signal,
        });
        responseBody = await response.text();
      } finally {
        clearTimeout(timer);
      }
      disposition = decode(module.fitness_classify_response(response.status, responseBody));
    } catch (_error) {
      disposition = decode(module.fitness_classify_response(0, ""));
    }
    const applied = decode(
      module.fitness_apply_response(JSON.stringify({ queued, disposition })),
    );
    if (JSON.stringify(applied.queued) !== JSON.stringify(queued)) {
      await writeOutbox(db, applied.queued);
      changed = true;
    }
    if (applied.auth_blocked) authBlocked = true;
    if (!applied.continue_flushing) {
      retryPending = !applied.auth_blocked;
      break;
    }
  }

  const stillPending =
    decode(module.fitness_pending_outbox(JSON.stringify(await readAllOutbox(db)))).length > 0;
  if (changed) await broadcastChange(sourceClientId);
  return {
    auth_blocked: authBlocked,
    retry_pending: retryPending,
    pending: stillPending,
  };
}

async function draftStatus(db) {
  const { draft } = await readState(db);
  const hasDraft = Boolean(
    draft &&
      (draft.title !== "Workout" ||
        (typeof draft.notes === "string" && draft.notes.trim() !== "") ||
        (Array.isArray(draft.exercises) && draft.exercises.length > 0)),
  );
  return { has_draft: hasDraft };
}

async function composeSnapshot(db, draft, guide, derived, extras = {}) {
  const module = await wasm();
  return {
    draft,
    guide,
    derived,
    outbox: decode(module.fitness_order_outbox(JSON.stringify(await readAllOutbox(db)))),
    ...extras,
  };
}

async function registerBackgroundSync() {
  try {
    if (self.registration.sync?.register) await self.registration.sync.register(SYNC_TAG);
  } catch (_error) {
    // Page startup/online/pageshow/visibility events are the fallback.
  }
}

async function broadcastChange(excludeClientId = null) {
  const clients = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  for (const client of clients) {
    if (client.id === excludeClientId) continue;
    client.postMessage({
      protocol: self.FITNESS_ENTRY_WASM.protocol,
      type: "fitness-entry-changed",
    });
  }
}

function localId() {
  const value = self.crypto?.randomUUID?.();
  if (!value) throw new Error("Secure local identities are unavailable.");
  return value;
}

function decode(json) {
  try {
    return JSON.parse(json);
  } catch (_error) {
    throw new Error("Fitness entry wasm returned malformed JSON.");
  }
}

let databasePromise = null;

function openDatabase() {
  if (databasePromise) return databasePromise;
  databasePromise = new Promise((resolve, reject) => {
    let request;
    try {
      request = indexedDB.open(DATABASE, DATABASE_VERSION);
    } catch (_error) {
      reject(new Error("IndexedDB is unavailable for Fitness entry."));
      return;
    }
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STATE_STORE)) {
        db.createObjectStore(STATE_STORE, { keyPath: "key" });
      }
      if (!db.objectStoreNames.contains(OUTBOX_STORE)) {
        const outbox = db.createObjectStore(OUTBOX_STORE, { keyPath: "queue_id" });
        outbox.createIndex("by_enqueued_at", "enqueued_at_ms", { unique: false });
      }
    };
    request.onblocked = () => reject(new Error("Fitness entry storage upgrade is blocked."));
    request.onerror = () => reject(new Error("IndexedDB could not open Fitness entry storage."));
    request.onsuccess = () => {
      request.result.onversionchange = () => {
        request.result.close();
        databasePromise = null;
      };
      resolve(request.result);
    };
  }).catch((error) => {
    databasePromise = null;
    throw error;
  });
  return databasePromise;
}

async function readState(db) {
  const tx = db.transaction(STATE_STORE, "readonly");
  const done = transactionDone(tx);
  const store = tx.objectStore(STATE_STORE);
  const [draftRow, guideRow] = await Promise.all([
    requestValue(store.get("draft")),
    requestValue(store.get("guide")),
  ]);
  await done;
  return { draft: draftRow?.value || null, guide: guideRow?.value || null };
}

async function requireState(db) {
  const state = await readState(db);
  if (!state.draft || !state.guide) {
    throw new Error("Fitness entry has not initialized on this device.");
  }
  return state;
}

async function writeState(db, draft, guide) {
  const tx = db.transaction(STATE_STORE, "readwrite");
  const done = transactionDone(tx);
  tx.objectStore(STATE_STORE).put({ key: "draft", value: draft });
  tx.objectStore(STATE_STORE).put({ key: "guide", value: guide });
  await done;
}

async function writeDraft(db, draft) {
  const tx = db.transaction(STATE_STORE, "readwrite");
  const done = transactionDone(tx);
  tx.objectStore(STATE_STORE).put({ key: "draft", value: draft });
  await done;
}

async function commitFinalization(db, draft, queued) {
  const tx = db.transaction([STATE_STORE, OUTBOX_STORE], "readwrite");
  const done = transactionDone(tx);
  tx.objectStore(STATE_STORE).put({ key: "draft", value: draft });
  tx.objectStore(OUTBOX_STORE).add(queued);
  await done;
}

async function commitRestore(db, draft, queueId) {
  const tx = db.transaction([STATE_STORE, OUTBOX_STORE], "readwrite");
  const done = transactionDone(tx);
  tx.objectStore(STATE_STORE).put({ key: "draft", value: draft });
  tx.objectStore(OUTBOX_STORE).delete(queueId);
  await done;
}

async function readAllOutbox(db) {
  const tx = db.transaction(OUTBOX_STORE, "readonly");
  const done = transactionDone(tx);
  const rows = await requestValue(tx.objectStore(OUTBOX_STORE).getAll());
  await done;
  return Array.isArray(rows) ? rows : [];
}

async function readOutbox(db, queueId) {
  if (typeof queueId !== "string") return null;
  const tx = db.transaction(OUTBOX_STORE, "readonly");
  const done = transactionDone(tx);
  const row = await requestValue(tx.objectStore(OUTBOX_STORE).get(queueId));
  await done;
  return row || null;
}

async function writeOutbox(db, queued) {
  const tx = db.transaction(OUTBOX_STORE, "readwrite");
  const done = transactionDone(tx);
  tx.objectStore(OUTBOX_STORE).put(queued);
  await done;
}

async function deleteOutbox(db, queueId) {
  const tx = db.transaction(OUTBOX_STORE, "readwrite");
  const done = transactionDone(tx);
  tx.objectStore(OUTBOX_STORE).delete(queueId);
  await done;
}

function requestValue(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error || new Error("IndexedDB request failed."));
  });
}

function transactionDone(transaction) {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = () => reject(transaction.error || new Error("IndexedDB transaction aborted."));
    transaction.onerror = () => reject(transaction.error || new Error("IndexedDB transaction failed."));
  });
}
