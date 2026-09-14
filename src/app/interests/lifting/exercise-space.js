// Rust supplies exact neighborhoods, scores, and load. A worker fits UMAP;
// this renderer rotates its 3D points and presents the full-dimensional data.
for (const root of document.querySelectorAll('[data-exercise-space]')) {
  try { initExerciseSpace(root); } catch (error) { console.error('Exercise space could not initialize', error); }
}

function initExerciseSpace(root) {
  const data = JSON.parse(root.dataset.exerciseSpace);
  const find = (selector) => root.querySelector(selector);
  const canvas = find('[data-space-canvas]');
  const ctx = canvas.getContext('2d');
  const stage = find('[data-space-stage]');
  const searchInput = find('[data-space-search]');
  const searchResults = find('[data-space-search-results]');
  const searchFeedback = find('[data-space-search-feedback]');
  const initialQuery = find('[data-space-form]').dataset.spaceQuery;
  const tooltip = find('[data-space-tooltip]');
  const nameIndex = new Map(data.exercises.map((exercise, index) => [exercise.name, index]));
  let radius = Math.max(0.1, ...data.points.map((point) => Math.hypot(...point.position)));
  const fallbackPositions = data.points.map((point) => [...point.position]);
  let layoutReady = false, layoutWorker;
  const fitScale = Math.max(1, ...data.exercises.map((exercise) => Math.abs(exercise.score)));
  let selected = data.selected;
  let hovered = null;
  let width = 1, height = 1, frame = null;
  let view = { yaw: -0.55, pitch: -0.35, zoom: 1 };
  let drawn = [];
  let colors;
  const pointers = new Map();
  let moved = false;
  let dragDistance = 0;
  let searchTimer, searchRequest, searchGeneration = 0;
  let searchRows = [];
  let historyRequest, historyGeneration = 0;

  async function loadHistory(item, page = 1, focus = false) {
    historyRequest?.abort();
    const generation = ++historyGeneration;
    const panel = find('[data-space-history]');
    panel.hidden = !item;
    panel.removeAttribute('aria-busy');
    panel.replaceChildren();
    if (!item) return;
    panel.setAttribute('aria-busy', 'true');
    panel.replaceChildren(find('[data-space-history-loading-template]').content.cloneNode(true));
    panel.querySelector('h3').textContent = item.name;
    const request = new AbortController(); historyRequest = request;
    try {
      const url = new URL('/fitness/exercises/history', location.href);
      url.searchParams.set('exercise', item.name);
      url.searchParams.set('history_page', page);
      const response = await fetch(url, {signal:request.signal,headers:{Accept:'text/html'}});
      if (!response.ok) throw new Error('History unavailable');
      const html = await response.text();
      if (generation !== historyGeneration) return;
      // Clone the same server-rendered section used for the initial page.
      const section = new DOMParser().parseFromString(html, 'text/html').querySelector('[data-space-history]');
      if (!section) throw new Error('History unavailable');
      const replacement = document.importNode(section, true);
      panel.replaceWith(replacement);
      if (focus) { replacement.tabIndex = -1; replacement.focus({preventScroll:true}); }
      const currentUrl = new URL(location.href);
      if (page > 1) currentUrl.searchParams.set('history_page', page); else currentUrl.searchParams.delete('history_page');
      history.replaceState(null, '', currentUrl);
    } catch (error) {
      if (request.signal.aborted || generation !== historyGeneration) return;
      panel.replaceChildren(find('[data-space-history-error-template]').content.cloneNode(true));
      const retry = panel.querySelector('a');
      retry.href = item.map_url;
    } finally {
      if (generation === historyGeneration) panel.removeAttribute('aria-busy');
    }
  }

  function applyLayout(result) {
    result.positions.forEach((position, index) => { data.points[index].position = position; });
    radius = Math.max(.1, ...result.positions.map((point) => Math.hypot(...point)));
    find('[data-space-map-status]').hidden = true;
    find('[data-space-layout-label]').textContent = result.overlap == null ? '3D projection' : 'Balanced 3D';
    find('[data-space-layout-note]').textContent = result.overlap == null ? 'Nearby matches use all 28 muscles. Distances between clusters are approximate.' : `${Math.round(result.overlap * 100)}% of closest neighbors retained in 3D. Distances between clusters are approximate.`;
    schedule();
  }

  function loadLayout() {
    layoutWorker?.terminate(); layoutWorker = null;
    const fallback = () => {
      applyLayout({positions:fallbackPositions,overlap:null});
      find('[data-space-layout-label]').textContent = 'Linear fallback';
      find('[data-space-layout-note]').textContent = `UMAP could not load. The linear fallback retains ${Math.round(data.linear_retained * 100)}% of variation; full-muscle comparisons remain available.`;
    };
    try {
      const worker = new Worker(root.dataset.spaceLayoutWorker);
      layoutWorker = worker;
      find('[data-space-map-status]').hidden = false;
      find('[data-space-map-status]').textContent = 'Arranging exercise neighborhoods…';
      worker.onmessage = ({data: result}) => {
        if (layoutWorker !== worker) return;
        worker.terminate(); layoutWorker = null;
        if (result.error) { fallback(); return; }
        layoutReady = true; applyLayout(result);
      };
      worker.onerror = () => { if (layoutWorker !== worker) return; worker.terminate(); layoutWorker = null; fallback(); };
      worker.postMessage({embedding:data.embedding,fallback:fallbackPositions,library:new URL(root.dataset.spaceLayoutLibrary, location.href).href});
    } catch { fallback(); }
  }

  function updateClearButton() {
    find('[data-space-overview]').hidden = selected == null && !searchInput.value.trim();
  }

  function closeSearch() {
    clearTimeout(searchTimer); searchRequest?.abort(); searchGeneration++;
    searchRows = []; searchResults.replaceChildren(); searchResults.hidden = true;
    searchFeedback.hidden = true;
    searchInput.setAttribute('aria-expanded', 'false');
    updateClearButton();
  }

  async function searchExercises(pickFirst = false) {
    closeSearch();
    const query = searchInput.value.trim();
    if (!query) return;
    const generation = searchGeneration;
    const request = new AbortController(); searchRequest = request;
    searchFeedback.textContent = 'Searching exercises…'; searchFeedback.hidden = false;
    try {
      const response = await fetch(`/fitness/exercises/search?q=${encodeURIComponent(query)}`, { signal: request.signal, headers: { Accept: 'application/json' } });
      if (!response.ok) throw new Error('Search unavailable');
      const found = await response.json();
      if (generation !== searchGeneration || query !== searchInput.value.trim()) return;
      searchRows = found.matches;
      for (const item of searchRows) {
        const node = find('[data-space-search-template]').content.firstElementChild.cloneNode(true);
        node.dataset.spaceSearchChoice = item.name; node.href = item.url;
        node.querySelector('.entry-picker-option__name').textContent = item.name;
        node.querySelector('.entry-picker-option__reason').textContent = item.meta;
        searchResults.append(node);
      }
      searchResults.hidden = searchRows.length === 0;
      searchInput.setAttribute('aria-expanded', String(searchRows.length > 0));
      searchFeedback.textContent = searchRows.length === 0 ? 'No matching exercises.' : `${searchRows.length}${found.total > searchRows.length ? ` of ${found.total}` : ''} matching exercises. Enter explores the first match.`;
      if (pickFirst && searchRows.length) {
        const index = nameIndex.get(searchRows[0].name);
        if (index === undefined) location.assign(searchRows[0].url); else choose(index);
      }
    } catch (error) {
      if (request.signal.aborted || generation !== searchGeneration) return;
      searchFeedback.textContent = 'Exercise search could not load. Try searching again.';
    }
  }

  function palette() {
    if (!ctx) return;
    const style = getComputedStyle(root);
    const value = (name) => style.getPropertyValue(`--color-${name}`).trim();
    const probe = document.createElement('canvas').getContext('2d');
    const rgb = (color) => { probe.clearRect(0, 0, 1, 1); probe.fillStyle = color; probe.fillRect(0, 0, 1, 1); return [...probe.getImageData(0, 0, 1, 1).data].slice(0, 3); };
    colors = { ink: value('ink'), muted: value('muted'), accent: value('oxide'), page: value('page'), positive: rgb(value('patina')), negative: rgb(value('steel')), neutral: rgb(value('muted')) };
  }

  function fitColor(score) {
    const target = score >= 0 ? colors.positive : colors.negative;
    const amount = Math.sqrt(Math.min(1, Math.abs(score) / fitScale));
    return `rgb(${target.map((value, i) => Math.round(colors.neutral[i] + (value - colors.neutral[i]) * amount)).join(',')})`;
  }

  const scoreText = (score) => `${score >= 0 ? '+' : '−'}${Math.abs(score).toFixed(1)}`;
  const pointText = (centi) => String(Math.round(centi / 10) / 10);
  function updateLoads(item) {
    find('[data-space-load-note]').textContent = item ? '2 sets at RPE 9 · added % of target' : 'Select an exercise to preview 2 sets at RPE 9.';
    const rows = [...root.querySelectorAll('[data-space-load-row]')];
    for (const row of rows) {
      const index = Number(row.dataset.spaceLoadRow);
      const muscle = data.muscles[index];
      const current = muscle.current_centi;
      const delta = item?.load_delta_centi[index] || 0;
      const reference = muscle.target_centi ?? muscle.usual_centi;
      const scaled = reference > 0;
      const currentWidth = scaled ? Math.min(100, current / reference * 100) : 0;
      const deltaWidth = scaled ? Math.min(100 - currentWidth, delta / reference * 100) : 0;
      const track = row.querySelector('.space-load-track');
      track.style.setProperty('--space-current', `${currentWidth}%`);
      track.style.setProperty('--space-delta', `${deltaWidth}%`);
      track.style.setProperty('--space-usual', `${scaled ? Math.min(100, muscle.usual_centi / reference * 100) : 0}%`);
      row.dataset.preview = String(delta > 0);
      row.dataset.unscaled = String(!scaled);
      row.querySelector('.space-load-usual').hidden = !scaled || muscle.usual_centi <= 0;
      row.querySelector('.space-load-overflow').hidden = !scaled || current + delta <= reference;
      const percentage = scaled ? `${item && delta > 0 ? '+' : ''}${Math.round((item ? delta : current) / reference * 100)}%` : delta > 0 ? `+${pointText(delta)} pt` : '—';
      row.querySelector('[data-space-load-percent]').textContent = percentage;
      const target = muscle.target_centi == null ? 'no weekly target set' : `weekly target ${pointText(muscle.target_centi)} points`;
      const description = `${pointText(current)} points in the past seven days; +${pointText(delta)} points from two sets at RPE 9; ${pointText(current + delta)} points after; ${target}; usual weekly pace ${pointText(muscle.usual_centi)} points.`;
      row.title = description;
      row.querySelector('[data-space-load-description]').textContent = description;
    }
    const fraction = (index) => {
      const muscle = data.muscles[index];
      const reference = muscle.target_centi ?? muscle.usual_centi;
      return reference > 0 && item ? item.load_delta_centi[index] / reference : 0;
    };
    rows.sort((left, right) => {
      const a = Number(left.dataset.spaceLoadRow), b = Number(right.dataset.spaceLoadRow);
      return fraction(b) - fraction(a) || Number((item?.load_delta_centi[b] || 0) > 0) - Number((item?.load_delta_centi[a] || 0) > 0) || a - b;
    });
    find('.space-load-rows').append(...rows);
  }

  function choose(index, updateUrl = true) {
    selected = index;
    data.selected = index;
    const item = index == null ? null : data.exercises[index];
    closeSearch();
    searchInput.value = item?.name || '';
    updateClearButton();
    find('[data-space-selected]').value = item?.name || '';
    find('[data-space-selection-label]').textContent = item ? 'Selected exercise' : 'Your training compass';
    find('[data-space-title]').textContent = item?.name || 'Best training fits';
    find('[data-space-score]').hidden = item?.point == null;
    find('[data-space-score]').dataset.sign = item?.score < 0 ? 'negative' : 'positive';
    find('[data-space-score-value]').textContent = item ? scoreText(item.score) : '';
    const link = find('[data-space-exercise-link]');
    link.hidden = !item;
    link.href = item?.url || '/fitness/exercises';
    const siblings = item?.point == null ? 0 : data.points[item.point].members.length;
    find('[data-space-shared]').hidden = siblings < 2;
    find('[data-space-shared]').textContent = `${siblings} exercises share this muscle and movement profile.`;
    find('[data-space-neighbors-title]').textContent = item ? 'Closest muscle matches' : 'Best matches for saved load';
    find('[data-space-neighbors-note]').textContent = item ? item.point == null ? 'Add a muscle profile in exercise details to see similar exercises.' : 'Similarity uses the complete muscle profile.' : 'Higher scores cover more of the current muscle gaps.';
    const rows = item ? item.neighbors : data.fit_order.slice(0, 6).map((index) => ({ index }));
    const list = find('[data-space-neighbors]');
    list.replaceChildren();
    for (const row of rows) {
      const candidate = data.exercises[row.index];
      const node = find('[data-space-neighbor-template]').content.firstElementChild.cloneNode(true);
      const anchor = node.querySelector('a');
      anchor.dataset.spaceChoice = row.index;
      anchor.href = candidate.map_url;
      anchor.querySelector('span').textContent = candidate.name;
      anchor.querySelector('strong').textContent = item ? (row.similarity >= 1 - 1e-10 ? '100%' : `${Math.min(99.9, row.similarity * 100).toFixed(1)}%`) : scoreText(candidate.score);
      list.append(node);
    }
    updateLoads(item);
    if (updateUrl) {
      loadHistory(item);
      const url = new URL(location.href);
      url.searchParams.delete('q');
      url.searchParams.delete('history_page');
      url.searchParams.delete('details');
      url.searchParams.delete('notice');
      url.hash = '';
      if (item) url.searchParams.set('exercise', item.name); else url.searchParams.delete('exercise');
      history.replaceState(null, '', url);
    }
    tooltip.hidden = true;
    schedule();
  }

  root.addEventListener('click', (event) => {
    const historyLink = event.target.closest('[data-space-history-page]');
    if (historyLink && !event.ctrlKey && !event.metaKey && !event.shiftKey && !event.altKey && event.button === 0) {
      event.preventDefault();
      loadHistory(selected == null ? null : data.exercises[selected], Number(historyLink.dataset.spaceHistoryPage), event.detail === 0);
      return;
    }
    const choice = event.target.closest('[data-space-choice], [data-space-search-choice], [data-space-overview]');
    if (!choice || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey || event.button !== 0) return;
    const index = choice.hasAttribute('data-space-overview') ? null : choice.hasAttribute('data-space-search-choice') ? nameIndex.get(choice.dataset.spaceSearchChoice) : Number(choice.dataset.spaceChoice);
    if (index === undefined) return;
    event.preventDefault();
    choose(index);
    if (event.detail === 0) searchInput.focus({ preventScroll: true });
  });
  searchInput.addEventListener('input', () => { closeSearch(); if (searchInput.value.trim()) searchTimer = setTimeout(() => searchExercises(), 120); });
  searchInput.addEventListener('keydown', (event) => {
    if (event.isComposing) return;
    if (event.key === 'ArrowDown' && !searchResults.hidden) { event.preventDefault(); searchResults.firstElementChild?.focus(); }
    if (event.key === 'Escape') { closeSearch(); searchInput.value = selected == null ? '' : data.exercises[selected].name; updateClearButton(); }
  });
  searchResults.addEventListener('keydown', (event) => {
    const nodes = [...searchResults.children];
    const index = nodes.indexOf(event.target.closest('a'));
    if (event.key === 'Escape') { closeSearch(); searchInput.focus(); }
    if (event.key === 'ArrowDown') { event.preventDefault(); nodes[Math.min(index + 1, nodes.length - 1)]?.focus(); }
    if (event.key === 'ArrowUp') { event.preventDefault(); if (index <= 0) searchInput.focus(); else nodes[index - 1].focus(); }
  });
  find('[data-space-form]').addEventListener('submit', (event) => { event.preventDefault(); searchExercises(true); });
  find('[data-space-reset]').hidden = false;
  find('[data-space-reset]').addEventListener('click', () => { view = { yaw: -0.55, pitch: -0.35, zoom: 1 }; schedule(); });
  window.addEventListener('popstate', () => { choose(nameIndex.get(new URL(location.href).searchParams.get('exercise')) ?? null, false); loadHistory(selected == null ? null : data.exercises[selected]); });
  document.addEventListener('exercise-details-select', (event) => {
    const index = nameIndex.get(event.detail.name);
    if (index !== undefined && index !== selected) choose(index);
  });

  function project(position) {
    return projectExercisePoint(position.map((value) => value / radius), view, width, height);
  }

  function schedule() {
    if (!ctx || frame != null) return;
    frame = requestAnimationFrame(() => { frame = null; draw(); });
  }

  function draw() {
    ctx.clearRect(0, 0, width, height);
    ctx.lineWidth = 1;
    const item = selected == null ? null : data.exercises[selected];
    const nearby = new Set(item ? item.neighbors.map((neighbor) => data.exercises[neighbor.index].point) : data.fit_order.slice(0, 6).map((index) => data.exercises[index].point));
    drawn = data.points.map((point, index) => {
      const member = item?.point === index ? selected : point.members.reduce((best, candidate) => data.exercises[candidate].score > data.exercises[best].score ? candidate : best, point.members[0]);
      return { ...project(point.position), index, member, selected: item?.point === index, near: nearby.has(index), count: point.members.length };
    });
    if (item?.point != null) {
      const from = drawn[item.point];
      ctx.strokeStyle = colors.accent;
      ctx.globalAlpha = .18;
      for (const index of nearby) { const to = drawn[index]; ctx.beginPath(); ctx.moveTo(from.x, from.y); ctx.lineTo(to.x, to.y); ctx.stroke(); }
    }
    drawn.sort((a, b) => b.depth - a.depth);
    for (const point of drawn) {
      const active = point.selected || point.near || point.index === hovered;
      point.r = (point.selected ? 7 : 3 + Math.min(3, Math.sqrt(point.count) * .55)) * point.scale;
      ctx.globalAlpha = item && !active ? .35 : .82;
      ctx.fillStyle = fitColor(data.exercises[point.member].score);
      ctx.beginPath(); ctx.arc(point.x, point.y, point.r, 0, Math.PI * 2); ctx.fill();
      if (point.selected || point.index === hovered) {
        ctx.globalAlpha = 1;
        ctx.lineWidth = point.selected ? 2 : 1;
        ctx.strokeStyle = point.selected ? colors.accent : colors.ink;
        ctx.beginPath(); ctx.arc(point.x, point.y, point.r + (point.selected ? 4 : 2), 0, Math.PI * 2); ctx.stroke();
      }
      if (point.count > 1 && point.r >= 4) {
        ctx.globalAlpha = .9; ctx.fillStyle = colors.page; ctx.font = '9px monospace'; ctx.textAlign = 'center'; ctx.textBaseline = 'middle'; ctx.fillText(String(point.count), point.x, point.y + .5);
      }
    }
    ctx.globalAlpha = .8;
    ctx.fillStyle = colors.muted;
    ctx.font = '11px monospace'; ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
    const used = [];
    for (const mark of data.landmarks) {
      const position = [0, 1, 2].map((axis) => mark.points.reduce((sum, index) => sum + data.points[index].position[axis], 0) / mark.points.length);
      const point = project(position);
      const labelWidth = ctx.measureText(mark.label).width;
      const x = Math.max(12, Math.min(width - labelWidth - 12, point.x + 14));
      const y = Math.max(65, Math.min(height - 70, point.y - 20));
      if (used.some((box) => Math.abs(box.y - y) < 18 && x < box.x + box.width + 12 && x + labelWidth + 12 > box.x)) continue;
      used.push({ x, y, width: labelWidth });
      ctx.fillText(mark.label, x, y);
    }
    ctx.globalAlpha = 1;
  }

  function resize() {
    const bounds = stage.getBoundingClientRect();
    width = bounds.width; height = bounds.height;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(width * dpr); canvas.height = Math.round(height * dpr);
    ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
    schedule();
  }
  function hit(event) {
    const bounds = canvas.getBoundingClientRect();
    const x = event.clientX - bounds.left, y = event.clientY - bounds.top;
    return drawn.filter((point) => Math.hypot(point.x - x, point.y - y) < Math.max(13, point.r + 5)).sort((a, b) => Math.hypot(a.x - x, a.y - y) - Math.hypot(b.x - x, b.y - y) || a.depth - b.depth)[0];
  }
  function clampView() { view.pitch = Math.max(-1.35, Math.min(1.35, view.pitch)); view.zoom = Math.max(.65, Math.min(2.5, view.zoom)); }
  canvas.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    moved = pointers.size > 1;
    dragDistance = 0;
    canvas.setPointerCapture(event.pointerId); canvas.dataset.dragging = 'true'; tooltip.hidden = true;
  });
  canvas.addEventListener('pointermove', (event) => {
    if (pointers.has(event.pointerId)) {
      const before = pointers.get(event.pointerId);
      const after = { x: event.clientX, y: event.clientY };
      if (pointers.size === 2) {
        const other = [...pointers.entries()].find(([id]) => id !== event.pointerId)[1];
        view.zoom *= Math.hypot(after.x - other.x, after.y - other.y) / Math.max(1, Math.hypot(before.x - other.x, before.y - other.y));
      } else {
        view.yaw += (after.x - before.x) * .008;
        view.pitch += (after.y - before.y) * .008;
      }
      dragDistance += Math.hypot(after.x - before.x, after.y - before.y);
      if (dragDistance > 4) moved = true;
      pointers.set(event.pointerId, after); clampView(); schedule(); return;
    }
    const point = hit(event);
    hovered = point?.index ?? null;
    tooltip.hidden = !point;
    if (point) {
      const exercise = data.exercises[point.member];
      tooltip.textContent = `${exercise.name}${point.count > 1 ? `\n${point.count} exercises share this profile` : ''}\nTraining fit ${scoreText(exercise.score)}`;
      tooltip.style.left = `${Math.max(8, Math.min(width - tooltip.offsetWidth - 8, point.x + 16))}px`;
      tooltip.style.top = `${Math.max(40, Math.min(height - tooltip.offsetHeight - 55, point.y + 14))}px`;
    }
    schedule();
  });
  const release = (event) => { pointers.delete(event.pointerId); if (!pointers.size) canvas.dataset.dragging = 'false'; };
  canvas.addEventListener('pointerup', (event) => {
    if (!moved && pointers.size === 1) { const point = hit(event); if (point) choose(point.member); }
    release(event);
  });
  canvas.addEventListener('pointercancel', release);
  canvas.addEventListener('lostpointercapture', release);
  canvas.addEventListener('pointerleave', () => { tooltip.hidden = true; hovered = null; schedule(); });
  canvas.addEventListener('wheel', (event) => { event.preventDefault(); view.zoom *= Math.exp(-event.deltaY * .001); clampView(); schedule(); }, { passive: false });
  for (const button of root.querySelectorAll('[data-space-turn]')) {
    button.addEventListener('click', () => {
      const action = button.dataset.spaceTurn;
      if (action === 'left') view.yaw -= .2;
      if (action === 'right') view.yaw += .2;
      if (action === 'up') view.pitch -= .15;
      if (action === 'down') view.pitch += .15;
      if (action === 'in') view.zoom *= 1.15;
      if (action === 'out') view.zoom /= 1.15;
      clampView(); schedule();
    });
  }
  palette();
  if (ctx) { find('[data-space-map-status]').hidden = true; find('[data-space-orbit]').hidden = false; }
  new ResizeObserver(resize).observe(stage);
  new MutationObserver(() => { palette(); schedule(); }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class', 'style'] });
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => { palette(); schedule(); });
  resize(); choose(selected, false);
  if (initialQuery) { searchInput.value = initialQuery; searchExercises(); }
  if (ctx) loadLayout();
  window.addEventListener('pagehide', () => { layoutWorker?.terminate(); layoutWorker = null; });
  window.addEventListener('pageshow', (event) => { if (event.persisted && ctx && !layoutReady) loadLayout(); });
}

function projectExercisePoint(position, view, width, height) {
  const [x, y, z] = position;
  const rotatedX = x * Math.cos(view.yaw) + z * Math.sin(view.yaw);
  const rotatedZ = -x * Math.sin(view.yaw) + z * Math.cos(view.yaw);
  const rotatedY = y * Math.cos(view.pitch) - rotatedZ * Math.sin(view.pitch);
  const depth = y * Math.sin(view.pitch) + rotatedZ * Math.cos(view.pitch);
  const scale = 3.6 / (3.6 + depth);
  const unit = Math.min(width, height) * .35 * view.zoom;
  return { x: width / 2 + rotatedX * unit * scale, y: height * .48 - rotatedY * unit * scale, depth, scale };
}
