// Run with just test-diary-browser. These execute the shipped page adapter
// and the actual Rust/Wasm session; only DOM and transport I/O are fixtures.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");
const root = path.resolve(__dirname, "../..");
const runtime = vm.createContext({
  console, TextEncoder, TextDecoder, WebAssembly, URL, setTimeout, clearTimeout,
  crypto: globalThis.crypto,
});
runtime.self = runtime;
runtime.bytes = fs.readFileSync(path.join(root, "wasm-dist/diary_sync_bg.wasm"));
vm.runInContext(fs.readFileSync(path.join(root, "wasm-dist/diary_sync.js"), "utf8"), runtime);
vm.runInContext("wasm_bindgen.initSync({module:bytes})", runtime);
const wasm = vm.runInContext("wasm_bindgen", runtime);
const source = fs.readFileSync(path.join(root, "src/app/diary/diary.js"), "utf8");

async function fixture(used = null) {
  let time = 0;
  const epoch = Date.parse("2026-09-10T12:00:00Z");
  let stored = used === null ? null : { day: "2026-09-10", body: "", used_ms: used,
    closed: false, closed_at: null, updated_at: epoch / 1000, revision: 1 };
  const cells = new Map();
  const element = () => ({ textContent: "", value: "", readOnly: false, hidden: false,
    disabled: false, dataset: {}, listeners: new Map(), setAttribute() {}, querySelector: () => null,
    focus() { this.focused = true; },
    addEventListener(name, fn) { this.listeners.set(name, [...(this.listeners.get(name) || []), fn]); },
    emit(name, data = {}) {
      const event = { prevented: false, preventDefault() { this.prevented = true; }, ...data };
      for (const fn of this.listeners.get(name) || []) fn(event);
      return event;
    } });
  const get = (id) => {
    if (!cells.has(id)) cells.set(id, element());
    return cells.get(id);
  };
  const root = get("diary-today");
  root.dataset = { day: "2026-09-10", reflection: JSON.stringify(stored) };
  root.querySelector = () => get("cell");
  const box = get("diary-reflection");
  const document = { ...element(), visibilityState: "visible", focused: true,
    hasFocus() { return this.focused; }, getElementById: get };
  let offline = false, drop = false, deferred = null, release;
  const writes = [];
  const snapshot = () => ({ schema_epoch: wasm.diary_schema_epoch(),
    days: stored ? [structuredClone(stored)] : [], emoji_usage: [] });
  async function request(effect) {
    if (deferred) { const wait = deferred; deferred = null; await wait; }
    if (offline) throw new Error("offline");
    const command = effect.command?.action;
    if (command) {
      writes.push(structuredClone(command));
      const replay = stored?.revision === command.expected_revision + 1 && stored.body === command.body
        && stored.used_ms === command.used_ms && stored.closed === command.close;
      if (!replay) {
        if ((stored?.revision || 0) !== command.expected_revision || stored?.closed
          || command.used_ms < (stored?.used_ms || 0)) throw new Error("conflict");
        stored = { day: command.day, body: command.body, used_ms: command.used_ms, closed: command.close,
          closed_at: command.close ? Math.floor((epoch + time) / 1000) : null,
          updated_at: Math.floor((epoch + time) / 1000), revision: command.expected_revision + 1 };
      }
      if (drop) { drop = false; throw new Error("offline"); }
    }
    return snapshot();
  }
  const window = element();
  const context = vm.createContext({
    console, document, window, navigator: { onLine: true, locks: { request: (_name, ...args) => args.at(-1)({}) } },
    Date: class extends Date { static now() { return epoch + time; } },
    performance: { now: () => time }, CSS: { escape: (value) => value },
    setInterval() {}, setTimeout, clearTimeout, Map, Set, Promise, request,
    fixtureWasm: { ...wasm, diary_now_cues: async () => JSON.stringify({ has_history: false, cues: [] }) },
  });
  vm.runInContext(source, context);
  vm.runInContext("ensureWasm = async () => fixtureWasm; todayRequest = request", context);
  const run = (code) => vm.runInContext(code, context);
  const settle = async () => {
    for (let n = 0; n < 5; n++) {
      await new Promise(setImmediate);
      const requests = run("[...todayEditor.requests]");
      if (!requests.length) return;
      await Promise.all(requests);
    }
    throw new Error("request loop");
  };
  await run("initToday()");
  await settle();
  return { get, box, document, window, writes, run, settle,
    stored: () => stored, view: () => run("todayEditor.view"),
    offline(value) { offline = value; }, dropResponse() { drop = true; },
    defer() { deferred = new Promise((resolve) => { release = resolve; }); }, release() { release(); },
    elsewhere() { stored = { ...stored, body: "Other device's saved reflection", used_ms: stored.used_ms + 2000, revision: stored.revision + 1 }; },
    async start() { get("diary-start").emit("click"); await settle(); },
    async advance(ms) { time += ms; run('dispatchToday({type:"tick"})'); await settle(); },
    type(body) {
      const event = box.emit("beforeinput");
      if (!event.prevented) { box.value = body; box.emit("input"); }
      return !event.prevented;
    },
    async close() { window.emit("blur"); await settle(); run("todayEditor.releaseLock?.(); todayEditor.session.free()"); },
  };
}

for (const [label, used, expected] of [["Start", null, "15:00"], ["Resume", 12000, "14:48"]]) {
  test(`${label} waits for a live response before opening the editor and charging time`, async () => {
    const ui = await fixture(used);
    assert.equal(ui.get("diary-start").textContent, label);
    assert.equal(ui.get("diary-writing").hidden, true);
    ui.defer();
    ui.get("diary-start").emit("click");
    ui.run('dispatchToday({type:"tick"})');
    assert.equal(ui.get("diary-writing").hidden, true);
    assert.equal(ui.get("diary-time").textContent, expected);
    ui.release();
    await ui.settle();
    assert.equal(ui.get("diary-writing").hidden, false);
    assert.equal(ui.box.focused, true);
    assert.equal(ui.view().phase, "Writing");
    for (let n = 0; n < 60; n++) await ui.advance(100);
    assert.equal(ui.stored().used_ms, (used || 0) + 5000);
    assert.equal(ui.view().phase, "Paused");
    await ui.close();
  });
}

test("blur, visibility loss, and beforeunload obey Rust's paused and unsaved state", async () => {
  const ui = await fixture(0);
  await ui.start();
  await ui.advance(250);
  ui.document.focused = false;
  ui.window.emit("blur");
  await ui.settle();
  await ui.advance(60_000);
  assert.equal(ui.stored().used_ms, 250);
  ui.document.focused = true;
  await ui.advance(60_000);
  assert.equal(ui.view().phase, "Paused");
  assert.equal(ui.stored().used_ms, 250);
  assert.equal(ui.type("  Exact text.\n\nSecond paragraph.  "), true);
  assert.equal(ui.window.emit("beforeunload").prevented, true);
  await ui.settle();
  assert.equal(ui.stored().body, ui.box.value);
  assert.equal(ui.window.emit("beforeunload").prevented, false);
  ui.document.visibilityState = "hidden";
  ui.document.emit("visibilitychange");
  await ui.advance(60_000);
  assert.equal(ui.box.readOnly, true);
  assert.equal(ui.stored().used_ms, 250);
  await ui.close();
});

test("a lost response freezes the editor and retries the same save without replacing newer text", async () => {
  const ui = await fixture(2000);
  await ui.start();
  ui.type("First words");
  ui.defer();
  const saving = ui.advance(400);
  ui.type("First words and more");
  ui.dropResponse();
  ui.release();
  await saving;
  const revision = ui.stored().revision;
  assert.equal(ui.box.readOnly, true);
  assert.equal(ui.box.value, "First words and more");
  assert.equal(ui.type("must be blocked"), false);
  ui.offline(true);
  await ui.advance(60_000);
  assert.equal(ui.stored().revision, revision);
  ui.offline(false);
  ui.run("refreshToday()");
  await ui.settle();
  // The first retry replays the unacknowledged command before sending the
  // newer text still held by the Rust session.
  assert.deepEqual(ui.writes[0], ui.writes[1]);
  await ui.advance(400);
  assert.equal(ui.stored().body, "First words and more");
  assert.equal(ui.view().phase, "Paused");
  assert.equal(ui.view().warn_before_leave, false);
  await ui.close();
});

test("Finish and expiry close the acknowledged entry and paint equal heatmap credit", async () => {
  for (const [used, finish] of [[0, true], [899000, false]]) {
    const ui = await fixture(used);
    await ui.start();
    ui.type("Last sentence.");
    if (finish) {
      ui.get("diary-finish").emit("click");
      await ui.settle();
    } else {
      for (let n = 0; n < 15; n++) await ui.advance(100);
    }
    assert.equal(ui.stored().closed, true);
    assert.equal(ui.stored().body, "Last sentence.");
    assert.equal(ui.box.readOnly, true);
    assert.equal(ui.get("diary-finish").hidden, true);
    assert.equal(ui.get("cell").dataset.status, "closed");
    assert.equal(ui.type("cannot reopen"), false);
    if (!finish) assert.equal(ui.get("diary-time").textContent, "0:00");
    await ui.close();
  }
});

test("offline Start stays closed and a concurrent device conflict preserves the visible draft", async () => {
  const ui = await fixture();
  ui.offline(true);
  await ui.start();
  assert.equal(ui.get("diary-writing").hidden, true);
  assert.equal(ui.stored(), null);
  ui.offline(false);
  await ui.start();
  ui.elsewhere();
  ui.type("My unsaved thought");
  await ui.advance(400);
  assert.equal(ui.box.readOnly, true);
  assert.equal(ui.box.value, "My unsaved thought");
  assert.equal(ui.stored().body, "Other device's saved reflection");
  assert.match(ui.view().status, /changed or closed elsewhere/);
  assert.equal(ui.view().warn_before_leave, true);
  await ui.close();
});
