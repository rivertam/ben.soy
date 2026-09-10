const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { pathToFileURL } = require('node:url');

class Element {
  constructor(tag) {
    this.tag = tag; this.dataset = {}; this.children = []; this.attributes = {};
    this.handlers = {}; this.id = ''; this.value = '';
    this.classList = { add() {}, remove() {} };
  }
  querySelector(selector) { return selector === 'input' ? this.children.find(n => n.tag === 'input') : null; }
  querySelectorAll(selector) { return selector === '[role=option]' ? this.children.filter(n => n.role === 'option') : []; }
  appendChild(child) { child.parent = this; this.children.push(child); }
  replaceChildren(...children) { this.children = children; for (const child of children) child.parent = this; }
  remove() { this.parent.children = this.parent.children.filter(child => child !== this); }
  setAttribute(name, value) { this.attributes[name] = value; }
  removeAttribute(name) { delete this.attributes[name]; }
  addEventListener(name, handler) { this.handlers[name] = handler; }
  blur() { this.handlers.blur?.(); }
  scrollIntoView() {}
}

test('the production combobox uses Rust Unicode search and keeps keyboard selection', async () => {
  const core = await import(pathToFileURL(path.resolve('wasm-dist/airport_search.js')));
  await core.default({ module_or_path: fs.readFileSync('wasm-dist/airport_search_bg.wasm') });
  for (const query of ['Cần Thơ', 'Can Tho', 'Ca\u0302\u0300n Tho\u031b']) {
    assert.equal(JSON.parse(core.airport_search(query))[0].iata, 'VCA');
  }
  assert.deepEqual(JSON.parse(core.airport_search('NYC')).slice(0, 3).map(a => a.iata), ['JFK', 'EWR', 'LGA']);
  const root = new Element('div');
  const input = new Element('input');
  input.id = 'airport-from';
  root.appendChild(input);
  const document = {
    querySelector: () => null,
    querySelectorAll: selector => selector === '[data-airport-combobox]' ? [root] : [],
    createElement: tag => new Element(tag),
  };
  // Only ES-module loading and DOM primitives are substituted. The shipped
  // combobox and the real generated Wasm execute together.
  const source = fs.readFileSync('src/app/thoughts/planes/airport-combobox.js', 'utf8')
    .replace("import { core } from '/airport-search.js'", '');
  vm.runInNewContext(source, { core, document });
  input.value = 'Cần Thơ';
  input.handlers.input();
  assert.equal(input.attributes['aria-expanded'], 'true');
  assert.equal(root.children[1].children[0].dataset.iata, 'VCA');
  input.handlers.keydown({ key: 'Enter', preventDefault() {} });
  assert.equal(input.value, 'VCA');
  assert.equal(root.children.length, 1);
  input.value = 'NYC';
  input.handlers.input();
  input.handlers.keydown({ key: 'ArrowDown', preventDefault() {} });
  assert.equal(input.attributes['aria-activedescendant'], 'airport-from-opt-1');
  input.handlers.keydown({ key: 'Enter', preventDefault() {} });
  assert.equal(input.value, 'EWR');
});
