const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { pathToFileURL } = require('node:url');

class Element {
  constructor() {
    this.dataset = {};
    this.handlers = {};
    this.nodes = new Map();
    this.textContent = '';
    this.hidden = false;
  }
  querySelector(selector) { return this.nodes.get(selector) ?? null; }
  querySelectorAll() { return []; }
  addEventListener(name, handler) { this.handlers[name] = handler; }
  matches() { return false; }
}

function serverRenderedForm(url, warningText) {
  const calculator = new Element();
  const form = new Element();
  const warning = new Element();
  warning.textContent = warningText;
  warning.hidden = !warningText;
  const rate = new Element();
  rate.value = '15';
  rate.validity = { valid: true };
  Object.defineProperty(rate, 'valueAsNumber', { get: () => Number(rate.value) });
  const scenario = new Element();
  scenario.value = 'wheat';
  scenario.dataset = {
    defaultRate: '15', yieldKg: '4000', rangeMin: '2', rangeMax: '100',
    crop: 'U.S. wheat', cropUnit: 'wheat', animalSingular: 'wood mouse', animalPlural: 'wood mice',
    meals: JSON.stringify([{ key: 'pasta', cropKg: 0.1, singular: 'bowl of pasta', plural: 'bowls of pasta' }]),
  };
  const deathRate = new Element();
  deathRate.textContent = '15';
  const receipt = new Element();
  receipt.textContent = '2,667';
  calculator.nodes.set('[data-crop-controls]', form);
  calculator.nodes.set('[data-crop-death-rate]', deathRate);
  calculator.nodes.set('[data-crop-result-units]', receipt);
  form.nodes.set('[data-crop-rate]', rate);
  form.nodes.set('[data-crop-warning]', warning);
  form.nodes.set('input[name="food"]:checked', scenario);
  form.nodes.set('input[name="meal"]:checked', { value: 'pasta' });
  const writes = [];
  const window = {
    location: { href: url },
    history: { replaceState(_state, _title, nextUrl) {
      writes.push(String(nextUrl));
      window.location.href = String(nextUrl);
    } },
  };
  const document = { querySelector: selector => selector === '[data-crop-calculator]' ? calculator : null };
  return { form, warning, rate, deathRate, receipt, document, window, writes };
}

test('enhancement preserves normalized SSR warnings and URLs until an actual edit', async () => {
  const core = await import(pathToFileURL(path.resolve('wasm-dist/thoughts_core.js')));
  await core.default({ module_or_path: fs.readFileSync('wasm-dist/thoughts_core_bg.wasm') });
  // Execute the shipped enhancement with real Wasm; only module loading and
  // the small DOM surface used by the form are substituted.
  const source = fs.readFileSync('src/app/thoughts/crop_deaths/crop-deaths.js', 'utf8')
    .replace('import { core } from "/thoughts-core.js";', '');
  for (const [query, warningText] of [
    ['food=wheat&rate=-1', 'Deaths per hectare per crop year must be positive; the calculator reset it to 15.'],
    ['food=wheat&meal=unknown', 'That meal isn’t mapped for this claim; showing bowls of pasta instead.'],
    ['food=wheat&rate=15', ''],
  ]) {
    const url = `https://example.test/thoughts/crop-deaths?${query}`;
    const fixture = serverRenderedForm(url, warningText);
    const { form, warning, rate, deathRate, receipt, document, window, writes } = fixture;
    vm.runInNewContext(source, { core, document, window, URL, Element, HTMLElement: Element });

    assert.equal(form.dataset.enhanced, 'true');
    assert.equal(warning.textContent, warningText, query);
    assert.equal(warning.hidden, !warningText);
    assert.equal(rate.value, '15');
    assert.equal(deathRate.textContent, '15');
    assert.equal(receipt.textContent, '2,667');
    assert.equal(window.location.href, url);
    assert.deepEqual(writes, [], 'startup must not rewrite the requested assumptions');

    rate.value = '1.005';
    form.handlers.input({ target: rate });
    assert.equal(warning.textContent, '');
    assert.equal(warning.hidden, true);
    assert.equal(deathRate.textContent, '1.01');
    assert.equal(receipt.textContent, core.crop_number(4000 / 1.005 / 0.1));
    assert.equal(new URL(window.location.href).searchParams.get('rate'), '1.005');
    assert.equal(new URL(window.location.href).searchParams.get('meal'), 'pasta');
    assert.equal(writes.length, 1);

    const acceptedReceipt = receipt.textContent;
    const acceptedUrl = window.location.href;
    rate.value = '-1';
    rate.validity.valid = false;
    form.handlers.input({ target: rate });
    assert.equal(warning.hidden, false);
    assert.match(warning.textContent, /Enter positive values/);
    assert.equal(receipt.textContent, acceptedReceipt);
    assert.equal(window.location.href, acceptedUrl);
    assert.equal(writes.length, 1);
  }
});
