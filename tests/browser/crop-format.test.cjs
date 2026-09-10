const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const { pathToFileURL } = require('node:url');
const path = require('node:path');

test('the shipped Rust formatter matches Intl at decimal ties and precision boundaries', async () => {
  const core = await import(pathToFileURL(path.resolve('wasm-dist/thoughts_core.js')));
  await core.default({module_or_path:fs.readFileSync('wasm-dist/thoughts_core_bg.wasm')});
  for (const value of [0, 0.001, 0.1005, 0.9995, 1, 1.005, 1.015, 9.995, 10, 10.25, 99.95, 100, 999.5, 99999.5, 1000000]) {
    const digits = value >= 100 ? 0 : value >= 10 ? 1 : value >= 1 ? 2 : 3;
    const expected = new Intl.NumberFormat('en-US', { maximumFractionDigits: digits }).format(value);
    assert.equal(core.crop_number(value), expected, String(value));
  }
  const receipt = JSON.parse(core.crop_calculation(4000, 2, 0.1));
  assert.deepEqual(receipt, {food_kg:2000, meal_count:20000, meals_per_hectare:40000});
  assert.throws(() => core.crop_calculation(4000, -1, 0.1));
});
