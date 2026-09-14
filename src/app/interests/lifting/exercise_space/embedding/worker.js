// Runs off the UI thread. Graph construction and exercise scoring live in Rust.
self.onmessage = ({ data }) => {
  try {
    importScripts(data.library);
    const { features, graph } = data.embedding;
    if (features.length < 4) {
      self.postMessage({ positions: data.fallback, overlap: null });
      return;
    }
    let seed = 0x5eeda11;
    const random = () => {
      seed = (seed + 0x6D2B79F5) | 0;
      let value = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      value = (value + Math.imul(value ^ (value >>> 7), 61 | value)) ^ value;
      return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
    };
    const model = new UMAP.UMAP({ nComponents: 3, nEpochs: 400, nNeighbors: graph.indices[0].length, minDist: 0.3, random });
    model.setPrecomputedKNN(graph.indices, graph.distances);
    const positions = model.fit(features);
    if (positions.some((point) => point.length !== 3 || point.some((value) => !Number.isFinite(value)))) throw new Error('Invalid embedding');
    // UMAP supplies the neighborhood arrangement. Metric MDS then fits the
    // original distances across the whole catalog to reduce artificial gaps
    // or overlaps between different movement groups.
    fitGlobalDistances(positions, graph.pair_distances, data.embedding.distance_scale);
    // Center the cloud without distorting relative distances along its axes.
    const mean = [0, 1, 2].map((axis) => positions.reduce((sum, point) => sum + point[axis], 0) / positions.length);
    for (const point of positions) for (let axis = 0; axis < 3; axis++) point[axis] -= mean[axis];
    const count = Math.min(10, graph.indices[0].length - 1);
    const distance = (left, right) => left.reduce((sum, value, axis) => sum + (value - right[axis]) ** 2, 0);
    let overlap = 0;
    for (let index = 0; index < positions.length; index++) {
      const expected = new Set(graph.indices[index].filter((other) => other !== index).slice(0, count));
      const projected = positions.map((point, other) => ({ other, distance: distance(positions[index], point) })).filter(({ other }) => other !== index).sort((a, b) => a.distance - b.distance || a.other - b.other).slice(0, count);
      overlap += projected.filter(({ other }) => expected.has(other)).length / count;
    }
    self.postMessage({ positions, overlap: overlap / positions.length });
  } catch (error) {
    self.postMessage({ error: String(error) });
  }
};

function fitGlobalDistances(positions, packedDistances, distanceScale) {
  const count = positions.length;
  if (packedDistances.length !== count * (count - 1) / 2 || !(distanceScale > 0)) throw new Error('Invalid global distances');
  const targets = Float64Array.from(packedDistances, value => value / distanceScale);
  // Find the least-squares uniform scale before fitting. UMAP's output units
  // are arbitrary; the input metric uses the same scale for every pair.
  let numerator = 0, denominator = 0, pair = 0;
  for (let i = 0; i < count; i++) for (let j = i + 1; j < count; j++) {
    const distance = Math.hypot(...positions[i].map((value, axis) => value - positions[j][axis]));
    numerator += distance * targets[pair++];
    denominator += distance * distance;
  }
  const scale = denominator > 0 ? numerator / denominator : 1;
  for (const point of positions) for (let axis = 0; axis < 3; axis++) point[axis] *= scale;
  // Complete, equal-weight SMACOF majorization: each iteration reduces metric
  // stress while retaining UMAP's initialization of the local neighborhoods.
  const next = new Float64Array(count * 3);
  for (let iteration = 0; iteration < 100; iteration++) {
    next.fill(0); pair = 0;
    for (let i = 0; i < count; i++) for (let j = i + 1; j < count; j++) {
      const x = positions[i][0] - positions[j][0], y = positions[i][1] - positions[j][1], z = positions[i][2] - positions[j][2];
      const ratio = targets[pair++] / Math.max(1e-12, Math.hypot(x, y, z));
      next[i * 3] += ratio * x; next[j * 3] -= ratio * x;
      next[i * 3 + 1] += ratio * y; next[j * 3 + 1] -= ratio * y;
      next[i * 3 + 2] += ratio * z; next[j * 3 + 2] -= ratio * z;
    }
    for (let i = 0; i < count; i++) for (let axis = 0; axis < 3; axis++) positions[i][axis] = next[i * 3 + axis] / count;
  }
}
