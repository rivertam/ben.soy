const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const directory = path.resolve(__dirname, '../../src/app/interests/lifting/exercise_space/embedding');
const workerSource = readFileSync(path.join(directory, 'worker.js'), 'utf8');
const librarySource = readFileSync(path.join(directory, 'umap-js-1.4.0.min.js'), 'utf8');

function project(features) {
  const messages = [];
  const graph = { indices: [], distances: [], pair_distances: [] };
  for (let index = 0; index < features.length; index++) {
    const neighbors = features.map((point, other) => ({other, distance:Math.hypot(...point.map((value, axis) => value - features[index][axis]))}))
      .sort((a, b) => a.distance - b.distance || Number(a.other !== index) - Number(b.other !== index) || a.other - b.other)
      .slice(0, Math.min(4, features.length - 1));
    graph.indices.push(neighbors.map(({other}) => other));
    graph.distances.push(neighbors.map(({distance}) => distance));
    for (let other = index + 1; other < features.length; other++) {
      graph.pair_distances.push(Math.round(Math.hypot(...features[index].map((value, axis) => value - features[other][axis])) * 10_000));
    }
  }
  const context = vm.createContext({
    self: { postMessage: (message) => messages.push(JSON.parse(JSON.stringify(message))) },
    importScripts: () => vm.runInContext(librarySource, context),
  });
  vm.runInContext(workerSource, context);
  context.self.onmessage({data:{library:'/umap.js',embedding:{features,graph,distance_scale:10_000},fallback:features.map(() => [0,0,0])}});
  assert.equal(messages.length, 1);
  assert.equal(messages[0].error, undefined);
  return messages[0];
}

test('the shipped UMAP worker keeps neighborhoods in deterministic finite 3D positions', () => {
  const features = Array.from({length:16}, (_, index) => {
    const cluster = Math.floor(index / 4), offset = index % 4 * .015;
    return [cluster === 0 ? 1 : offset, cluster === 1 ? 1 : offset, cluster === 2 ? 1 : offset, cluster === 3 ? 1 : offset];
  });
  const first = project(features);
  assert.equal(first.positions.length, features.length);
  assert.ok(first.positions.every((point) => point.length === 3 && point.every(Number.isFinite)));
  assert.ok(first.overlap >= .65, `neighbor overlap ${first.overlap}`);
  assert.deepEqual(first, project(features));
});

test('tiny catalogs retain their finite fallback without trying to fit UMAP', () => {
  for (const features of [[], [[1,0]], [[1,0],[0,1]], [[1,0],[0,1],[.5,.5]]]) {
    const result = project(features);
    assert.deepEqual(result.positions, features.map(() => [0,0,0]));
    assert.equal(result.overlap, null);
  }
});

test('separated neighborhoods retain their relative spacing across the full map', () => {
  const centers = [[0,0,0], [1,0,0], [0,3,0], [1,3,0]];
  const features = centers.flatMap(center => Array.from({length:4}, (_, i) => center.map((value, axis) => value + (axis === 2 ? i * .015 : 0))));
  const {positions} = project(features);
  const distance = (a, b) => Math.hypot(...a.map((value, axis) => value - b[axis]));
  let error = 0, total = 0;
  for (let i = 0; i < features.length; i++) for (let j = i + 1; j < features.length; j++) {
    const expected = distance(features[i], features[j]);
    error += (distance(positions[i], positions[j]) - expected) ** 2;
    total += expected ** 2;
  }
  assert.ok(Math.sqrt(error / total) < .1, `whole-map distance error ${Math.sqrt(error / total)}`);
  assert.ok(distance(positions[0], positions[4]) < distance(positions[0], positions[8]) / 2, 'nearby groups stay closer than distant groups');
});

test('a shared support profile can occupy the interior between compound groups', () => {
  const features = Array.from({length:24}, (_, i) => Array.from({length:7}, (_, axis) => axis === 6 ? .5 : axis === Math.floor(i / 4) ? 1 + (i % 4) * .01 : 0));
  features.push([0,0,0,0,0,0,1]);
  const {positions} = project(features);
  const center = [0,1,2].map(axis => positions.reduce((sum, point) => sum + point[axis], 0) / positions.length);
  const radius = point => Math.hypot(...point.map((value, axis) => value - center[axis]));
  const compoundRadii = positions.slice(0,24).map(radius).sort((a,b) => a-b);
  assert.ok(radius(positions[24]) < compoundRadii[12] * .8, 'shared support belongs inside the compound groups, not on their outer shell');
});
