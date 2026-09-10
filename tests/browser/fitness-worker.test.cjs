const { test, before } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

let core;
before(async () => {
  const context = vm.createContext({ TextDecoder, TextEncoder, WebAssembly, console });
  vm.runInContext(fs.readFileSync("wasm-dist/fitness_entry.js", "utf8"), context);
  core = vm.runInContext("wasm_bindgen", context);
  context.wasmBytes = fs.readFileSync("wasm-dist/fitness_entry_bg.wasm");
  await vm.runInContext("wasm_bindgen({ module_or_path: wasmBytes })", context);
});

const draft = () => ({
  version: 1, started_at_utc: "2026-09-03 14:00:00", title: "Workout", notes: "",
  exercises: [{ id: "exercise-0001", name: "Squat", sets: [{
    id: "set-00000001", weight: "100", reps: "10", effort: "9",
    failure: false, set_type: "NORMAL_SET", done: true,
  }] }],
});
const guide = () => ({ version: 1, today: "2026-09-03", weekly_pace_tenths: 30, muscle_needs: {}, exercises: [] });
const finalization = (queue_id, enqueued_at_ms) => JSON.parse(core.fitness_finalize(JSON.stringify({
  draft: draft(), guide: guide(), ended_at_utc: "2026-09-03 15:00:00",
  queue_id, enqueued_at_ms, context: {},
})));
const deferred = () => {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
};
const turn = () => new Promise(setImmediate);

function worker() {
  const handlers = new Map();
  const outbox = new Map();
  const state = { draft: draft(), guide: guide() };
  const requests = [];
  const broadcasts = [];
  let commitGate = Promise.resolve();
  const context = vm.createContext({
    console, AbortController, JSON,
    setTimeout: (...args) => { const timer = setTimeout(...args); timer.unref(); return timer; },
    clearTimeout,
    importScripts() {},
    wasm_bindgen: Object.assign(async () => {}, core),
    self: {
      FITNESS_ENTRY_WASM: { glue: "test-glue", wasm: "test-wasm", protocol: core.fitness_protocol_version() },
      addEventListener: (name, handler) => handlers.set(name, handler),
      crypto: { randomUUID: () => "queue-new-0001" },
      registration: { sync: { register: async () => {} } },
      clients: { matchAll: async () => [{ id: "page-1", postMessage: (value) => broadcasts.push(value) }] },
    },
    fetch: (path, options) => {
      const result = deferred();
      requests.push({ path, options, respond: (status, body = "") => result.resolve({ status, text: async () => body }) });
      return result.promise;
    },
  });
  vm.runInContext(fs.readFileSync("src/app/running/sw.js", "utf8"), context);
  // Replace only the IndexedDB adapter: all worker orchestration and Rust
  // transitions execute exactly as shipped, while commits can be held open.
  Object.assign(context, {
    openDatabase: async () => ({}),
    readState: async () => structuredClone(state),
    readAllOutbox: async () => structuredClone([...outbox.values()]),
    readOutbox: async (_, id) => structuredClone(outbox.get(id)),
    writeDraft: async (_, next) => { state.draft = structuredClone(next); },
    writeOutbox: async (_, row) => outbox.set(row.queue_id, structuredClone(row)),
    commitFinalization: async (_, next, queued) => {
      await commitGate;
      state.draft = structuredClone(next);
      outbox.set(queued.queue_id, structuredClone(queued));
    },
  });
  let requestId = 0;
  return { outbox, state, requests, broadcasts,
    holdCommit: (promise) => { commitGate = promise; },
    rpc(method, payload = {}) {
      const reply = deferred();
      let lifetime;
      handlers.get("message")({
        data: { protocol: core.fitness_protocol_version(), request_id: String(++requestId), method, payload },
        source: { id: "page-1" }, ports: [{ postMessage: reply.resolve }],
        waitUntil: (work) => { lifetime = work; },
      });
      return { reply: reply.promise, lifetime };
    },
  };
}

test("finalization acknowledges its commit while multiple uploads remain in flight", { timeout: 5000 }, async () => {
  const w = worker();
  // Intentionally insert in reverse order; the Rust queue selects oldest first.
  for (const [id, time] of [["queue-old-0002", 2], ["queue-old-0001", 1]]) {
    const row = finalization(id, time).queued;
    w.outbox.set(id, row);
  }
  const committed = deferred();
  w.holdCommit(committed.promise);
  const finish = w.rpc("finalize", { ended_at_utc: "2026-09-03 15:00:00", enqueued_at_ms: 3 });
  let replied = false;
  finish.reply.then(() => { replied = true; });
  await turn();
  assert.equal(replied, false);
  assert.equal(w.requests.length, 0);
  committed.resolve();
  const reply = await finish.reply;
  assert.equal(reply.ok, true);
  assert.equal(reply.value.enqueued_queue_id, "queue-new-0001");
  assert.equal(w.state.draft.exercises.length, 0);
  assert.equal(w.outbox.size, 3);
  await turn();
  assert.equal(w.requests.length, 1);

  const edit = await w.rpc("transition", { action: { type: "set_title", value: "Next workout" } }).reply;
  assert.equal(edit.ok, true);
  assert.equal(w.state.draft.title, "Next workout");
  const kick = w.rpc("flush");
  assert.equal((await kick.reply).ok, true);
  assert.equal(w.requests.length, 1, "overlapping kicks share the same upload");

  for (let index = 0; index < 3; index++) {
    assert.equal(w.requests.length, index + 1);
    w.requests[index].respond(422, '{"error":"test rejection"}');
    await turn();
    assert.equal([...w.outbox.values()].filter((row) => row.state === "failed").length, index + 1);
    if (index === 0) assert.equal(w.outbox.get("queue-old-0001").state, "failed");
  }
  await Promise.all([finish.lifetime, kick.lifetime]);
  assert.equal(w.state.draft.title, "Next workout");
  assert.ok(w.broadcasts.length > 0, "the initiating page receives publication changes");
});

test("retryable upload failures keep exact queued bytes for a later pass", { timeout: 5000 }, async () => {
  const w = worker();
  const row = finalization("queue-old-0001", 1).queued;
  w.outbox.set(row.queue_id, row);
  const kick = w.rpc("flush_only");
  assert.equal((await kick.reply).ok, true);
  await turn();
  const body = w.requests[0].options.body;
  w.requests[0].respond(503);
  await kick.lifetime;
  assert.deepEqual(w.outbox.get(row.queue_id), row);
  const retry = w.rpc("flush_only");
  await retry.reply;
  await turn();
  assert.equal(w.requests[1].options.body, body);
  w.requests[1].respond(401);
  await retry.lifetime;
  assert.deepEqual(w.outbox.get(row.queue_id), row);
});
