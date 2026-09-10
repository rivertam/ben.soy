// One progressive form for standalone setup and the entry dialog. Domain
// inference and markup remain server-owned; this adapter only moves the form.
function enhance(form) {
  if (!form) return;
  const step = Number(form.dataset.wizardStep || 1);
  const inEntry = Boolean(form.closest('[data-entry-create-dialog]'));
  for (const panel of form.querySelectorAll('[data-wizard-panel]')) {
    panel.hidden = Number(panel.dataset.wizardPanel) !== step;
  }
  for (const back of form.querySelectorAll('[data-wizard-back]')) back.hidden = false;
  const close = form.querySelector('[data-wizard-close]');
  if (close) close.hidden = !inEntry;
  const save = form.querySelector('[data-wizard-save]');
  if (save && inEntry) save.textContent = 'Create and add';
  const quickSave = form.querySelector('[data-wizard-quick-save]');
  if (quickSave && inEntry) quickSave.textContent = 'Create and add with suggested weights';
  const nameOnly = form.querySelector('[value="name_only"]');
  if (nameOnly && inEntry) nameOnly.textContent = 'Create and add name only';
  form.querySelector('[data-wizard-progress]').textContent = `${step} of 3 · ${['Name', 'Movement', 'Muscles'][step - 1]}`;
}

for (const form of document.querySelectorAll('[data-exercise-wizard]')) enhance(form);
document.addEventListener('exercise-wizard-open', (event) => enhance(event.target.querySelector('[data-exercise-wizard]')));

document.addEventListener('click', (event) => {
  const control = event.target.closest('[data-wizard-back], [data-wizard-close]');
  const form = control?.closest('[data-exercise-wizard]');
  if (!form) return;
  if (control.hasAttribute('data-wizard-close')) {
    form.closest('dialog')?.close();
  } else {
    form.dataset.wizardStep = control.dataset.wizardBack;
    enhance(form);
    form.querySelector('[data-wizard-panel]:not([hidden]) input')?.focus();
  }
});

document.addEventListener('submit', (event) => {
  const form = event.target.closest('[data-exercise-wizard]');
  if (!form) return;
  event.preventDefault();
  if (form.dataset.busy === 'true') return;
  void submit(form, event.submitter);
});

async function submit(form, submitter) {
  const status = form.querySelector('[data-wizard-status]');
  const intent = submitter?.value || 'save';
  const preview = intent === 'classify' || intent === 'suggest';
  const body = new URLSearchParams(new FormData(form));
  body.set('intent', intent);
  form.dataset.busy = 'true';
  const buttons = [...form.querySelectorAll('button')];
  buttons.forEach((button) => { button.disabled = true; });
  status.textContent = preview ? 'Finding a starting point…' : 'Saving exercise…';
  try {
    let saved = form._savedExercise;
    if (!saved) {
      const response = await fetch(submitter?.getAttribute('formaction') || form.action, {
        method: 'POST', credentials: 'same-origin', body,
        headers: { Accept: preview ? 'text/html' : 'application/json' },
      });
      if (!response.ok) throw new Error(await response.text() || 'The exercise could not be saved. Try again.');
      if (preview) {
        const documentResult = new DOMParser().parseFromString(await response.text(), 'text/html');
        const next = documentResult.querySelector('[data-exercise-wizard]');
        if (!next) throw new Error('The setup form could not load. Sign in again and retry.');
        form.replaceWith(next);
        enhance(next);
        next.querySelector('[data-wizard-panel]:not([hidden]) input')?.focus();
        return;
      }
      saved = await response.json();
      if (typeof saved.name !== 'string' || !saved.location?.startsWith('/fitness/exercise/')) throw new Error('The save response was incomplete. Retry to check the saved exercise.');
      form._savedExercise = saved;
    }
    status.textContent = saved.created ? 'Exercise saved.' : 'Using the existing exercise.';
    const detail = { saved, guide: null, completion: null };
    if (form.closest('[data-entry-create-dialog]')) {
      const response = await fetch('/fitness/entry/guide', { credentials: 'same-origin', headers: { Accept: 'application/json' } });
      if (!response.ok) throw new Error('Exercise saved. Retry adding it to your workout.');
      detail.guide = await response.json();
    }
    form.dispatchEvent(new CustomEvent('exercise-created', { bubbles: true, detail }));
    if (detail.completion) {
      await detail.completion;
      form.closest('dialog')?.close();
    } else {
      window.location.assign(saved.location);
    }
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : 'Could not save. Try again.';
    if (form._savedExercise) {
      status.textContent = 'Exercise saved. Retry adding it to your workout.';
      if (submitter) submitter.textContent = 'Retry adding';
    }
  } finally {
    form.dataset.busy = 'false';
    buttons.forEach((button) => { button.disabled = Boolean(form._savedExercise) && button !== submitter && !button.hasAttribute('data-wizard-close'); });
    if (form._savedExercise) for (const input of form.querySelectorAll('input, select')) input.disabled = true;
  }
}
