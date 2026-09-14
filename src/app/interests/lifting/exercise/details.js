// The server owns exercise definitions and markup; this adapter opens the
// inspector and moves existing rows between selected values and the picker.
const dialog = document.querySelector('[data-exercise-details-dialog]');
if (dialog) initDetails(dialog);

function initDetails(dialog) {
  const body = dialog.querySelector('[data-exercise-details-body]');
  const close = dialog.querySelector('[data-exercise-details-close]');
  let request, generation = 0, trigger;
  const initiallyOpen = dialog.open;
  if (initiallyOpen) dialog.removeAttribute('open');

  function selectionUrl(name, opened = true) {
    const url = new URL(location.href);
    url.pathname = '/fitness/exercises';
    url.searchParams.set('exercise', name);
    url.searchParams.delete('notice');
    if (opened) url.searchParams.set('details', '1'); else url.searchParams.delete('details');
    return url;
  }

  function install(content) {
    body.replaceChildren(document.importNode(content, true));
    enhanceEditor(body);
    const name = content.dataset.exerciseName;
    if (name) {
      document.dispatchEvent(new CustomEvent('exercise-details-select', {detail:{name}}));
      history.replaceState(null, '', selectionUrl(name));
      close.href = selectionUrl(name, false);
    }
    body.querySelector('[data-details-heading]')?.focus({preventScroll:true});
  }

  async function open(name, source) {
    trigger = source || trigger;
    request?.abort();
    const current = ++generation;
    request = new AbortController();
    const signal = request.signal;
    document.dispatchEvent(new CustomEvent('exercise-details-select', {detail:{name}}));
    history.replaceState(null, '', selectionUrl(name));
    close.href = selectionUrl(name, false);
    body.replaceChildren(document.querySelector('[data-exercise-details-loading]')?.content.cloneNode(true) || document.createTextNode('Loading exercise…'));
    dialog.setAttribute('aria-busy', 'true');
    if (!dialog.open) dialog.showModal();
    try {
      const url = new URL('/fitness/exercises/details', location.href);
      url.searchParams.set('exercise', name);
      const response = await fetch(url, {signal, credentials:'same-origin', headers:{Accept:'text/html'}});
      if (signal.aborted || current !== generation) return;
      const html = new DOMParser().parseFromString(await response.text(), 'text/html');
      if (signal.aborted || current !== generation) return;
      const content = html.querySelector('[data-exercise-details-content]');
      if (!response.ok || !content) throw new Error('Exercise details could not load. Close and try again.');
      install(content);
    } catch (error) {
      if (!signal.aborted && current === generation) body.textContent = error.message;
    } finally {
      if (current === generation) dialog.removeAttribute('aria-busy');
    }
  }

  document.addEventListener('click', (event) => {
    const link = event.target.closest('[data-exercise-details-link]');
    if (!link || event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    const name = new URL(link.href).searchParams.get('exercise');
    if (!name) return;
    event.preventDefault();
    void open(name, link);
  });
  close.addEventListener('click', (event) => { event.preventDefault(); dialog.close(); });
  dialog.addEventListener('close', () => {
    request?.abort(); generation++;
    dialog.removeAttribute('aria-busy');
    const url = new URL(location.href);
    if (url.pathname !== '/fitness/exercises') { location.assign(close.href); return; }
    url.searchParams.delete('details'); url.searchParams.delete('notice');
    history.replaceState(null, '', url);
    trigger?.focus({preventScroll:true});
  });
  window.addEventListener('popstate', () => {
    const url = new URL(location.href);
    if (url.searchParams.get('details') === '1' && url.searchParams.get('exercise')) void open(url.searchParams.get('exercise'));
    else if (dialog.open) dialog.close();
  });
  dialog.addEventListener('submit', (event) => {
    const form = event.target.closest('[data-exercise-definition-form], [data-exercise-identity-form]');
    if (!form) return;
    event.preventDefault();
    if (form.dataset.busy !== 'true') void save(form);
  });

  async function save(form) {
    const identity = form.hasAttribute('data-exercise-identity-form');
    const status = form.querySelector('[data-editor-status]');
    const payload = new URLSearchParams(new FormData(form));
    const controls = [...form.querySelectorAll('button')];
    form.dataset.busy = 'true'; controls.forEach((button) => { button.disabled = true; });
    status.textContent = identity ? 'Checking name and aliases…' : 'Saving changes…';
    try {
      const response = await fetch(form.action, {method:'POST',credentials:'same-origin',body:payload,headers:{Accept:identity ? 'text/html' : 'application/json','X-Exercise-Dialog':'1'}});
      if (!response.ok) throw new Error(await response.text() || 'Changes could not be saved. Try again.');
      if (!identity) {
        const saved = await response.json();
        if (typeof saved.name !== 'string') throw new Error('The save response was incomplete. Retry to check the saved exercise.');
        const url = selectionUrl(saved.name);
        url.searchParams.set('notice', 'saved');
        location.assign(url);
        return;
      }
      if (response.redirected) {
        const url = new URL(response.url);
        if (url.pathname !== '/fitness/exercises' || url.searchParams.get('notice') !== 'identity-saved') {
          throw new Error(url.searchParams.get('notice') === 'identity-stale' ? 'The archive changed. Review the name and aliases again.' : 'Changes were not saved. Check your sign-in and try again.');
        }
        const destination = selectionUrl(url.searchParams.get('exercise'));
        destination.searchParams.set('notice', 'identity-saved');
        location.assign(destination);
        return;
      }
      const content = new DOMParser().parseFromString(await response.text(), 'text/html').querySelector('[data-exercise-details-content]');
      if (!content) throw new Error('The review could not load. Try again.');
      install(content);
    } catch (error) {
      status.textContent = error.message || 'Changes could not be saved. Try again.';
    } finally {
      form.dataset.busy = 'false'; controls.forEach((button) => { button.disabled = false; });
    }
  }

  if (initiallyOpen) { dialog.showModal(); enhanceEditor(body); body.querySelector('[data-details-heading]')?.focus({preventScroll:true}); }
}

function enhanceEditor(body) {
  body.dataset.editorEnhanced = 'true';
  const form = body.querySelector('[data-exercise-definition-form]');
  if (!form) return;
  function updateDiagram() {
    const weights = new Map([...form.querySelectorAll('[data-editor-group="muscle"] [data-editor-selected] [data-editor-item]')].map((row) => [row.dataset.editorItem, Number(row.querySelector('input')?.value || row.querySelector('strong')?.textContent || 0)]));
    for (const path of body.querySelectorAll('.exercise-details__body-map [data-muscle]')) {
      const weight = weights.get(path.dataset.muscle) || 0;
      path.dataset.weight = weight >= 75 ? 'primary' : weight > 0 ? 'secondary' : 'none';
    }
  }
  for (const group of form.querySelectorAll('[data-editor-group]')) {
    const selected = group.querySelector('[data-editor-selected]');
    const available = group.querySelector('[data-editor-available]');
    if (!available) continue;
    const search = group.querySelector('[data-editor-search]');
    search.hidden = false;
    function refresh() {
      const terms = search.value.toLowerCase().trim().split(/\s+/).filter(Boolean);
      for (const row of group.querySelectorAll('[data-editor-item]')) {
        const active = row.parentElement === selected;
        const input = row.querySelector('[data-editor-value]');
        input.disabled = !active;
        if (input.type === 'checkbox') input.checked = active;
        row.querySelector('[data-editor-add]').hidden = active;
        row.querySelector('[data-editor-remove]').hidden = !active;
        row.hidden = !active && !terms.every((term) => row.dataset.editorTerms.toLowerCase().includes(term));
      }
      group.querySelector('[data-editor-no-matches]').hidden = [...available.children].some((row) => !row.hidden);
      updateDiagram();
    }
    group.addEventListener('click', (event) => {
      const button = event.target.closest('[data-editor-add], [data-editor-remove]');
      if (!button) return;
      const row = button.closest('[data-editor-item]');
      const adding = button.hasAttribute('data-editor-add');
      (adding ? selected : available).append(row);
      const input = row.querySelector('[data-editor-value]');
      if (adding && input.type === 'number' && Number(input.value) <= 0) input.value = '100';
      refresh();
      if (adding) {
        if (input.type === 'number') { input.focus(); input.select(); } else row.querySelector('[data-editor-remove]').focus();
      } else group.querySelector('summary').focus();
    });
    search.addEventListener('input', refresh);
    search.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter') return;
      event.preventDefault();
      [...available.children].find((row) => !row.hidden)?.querySelector('[data-editor-add]').click();
    });
    group.querySelector('details').addEventListener('toggle', (event) => { if (event.target.open) search.focus(); });
    refresh();
  }
  form.addEventListener('input', updateDiagram);
  updateDiagram();
}
