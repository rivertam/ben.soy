// All parsing, generation and drawing belong to Rust. This adapter edits
// served controls, formats bands, and ignores superseded preview responses.
export function notation(bands, mirrored) {
  const text = bands.map((band, i) => `${band.color}${mirrored && (i === 0 || i === bands.length - 1) ? "/" : ""}${band.threads}`).join(" ");
  return mirrored ? text : `...${text}...`;
}

export function latestRequest() {
  let revision = 0;
  return { next: () => ++revision, isCurrent: (value) => value === revision };
}

export function mount(root) {
  const $ = (selector) => root.querySelector(selector);
  const $$ = (selector) => [...root.querySelectorAll(selector)];
  const text = $("[data-plaid-document]");
  const status = $("[data-plaid-status]");
  const save = $("[data-plaid-save]");
  const seed = $("[data-plaid-seed]");
  let model = { spec: JSON.parse(text.value), warp: JSON.parse(root.dataset.warp), weft: JSON.parse(root.dataset.weft) };
  let savedText = JSON.stringify(model.spec);
  let revision = Number(root.dataset.revision);
  const generatorVersion = Number(root.dataset.generatorVersion);
  let valid = true;
  let busy = false;
  let timer;
  let abort;
  let lastGeneration = null;
  const requests = latestRequest();

  function message(value) { status.textContent = value; }
  function saveState() { save.disabled = busy || !valid || JSON.stringify(model.spec) === savedText; }
  function invalidate() {
    clearTimeout(timer);
    abort?.abort();
    requests.next();
    valid = false;
    saveState();
  }

  function colorRow(code, hex) {
    const row = $("[data-plaid-color-template]").content.firstElementChild.cloneNode(true);
    row.dataset.code = code;
    row.querySelector("[data-plaid-color-name]").textContent = code;
    const picker = row.querySelector("[data-plaid-color-value]");
    picker.value = hex;
    picker.setAttribute("aria-label", `${code} color`);
    row.querySelector("[data-plaid-remove-color]").setAttribute("aria-label", `Remove color ${code}`);
    return row;
  }

  function bandRow(band, codes) {
    const row = $("[data-plaid-band-template]").content.firstElementChild.cloneNode(true);
    const select = row.querySelector("[data-plaid-band-color]");
    for (const code of codes) select.add(new Option(code, code));
    select.value = band.color;
    row.querySelector("[data-plaid-band-width]").value = band.threads;
    return row;
  }

  function renderControls() {
    $("[data-plaid-palette]").replaceChildren(...Object.entries(model.spec.palette).map(([code, hex]) => colorRow(code, hex)));
    for (const axis of ["warp", "weft"]) {
      $(`[data-plaid-repeat="${axis}"]`).value = model[axis].mirrored ? "mirrored" : "repeating";
      $(`[data-plaid-bands="${axis}"]`).replaceChildren(...model[axis].bands.map(band => bandRow(band, Object.keys(model.spec.palette))));
    }
    $("[data-plaid-linked]").checked = !model.spec.weft;
    $("[data-plaid-axis=weft]").hidden = !model.spec.weft;
    $("[data-plaid-size]").value = model.spec.repeat_px;
    $("[data-plaid-angle]").value = model.spec.rotation_deg;
  }

  function readControls() {
    const palette = Object.fromEntries($$("[data-plaid-palette] [data-code]").map(row => [row.dataset.code, row.querySelector("input").value]));
    const axis = (name) => notation([...$(`[data-plaid-bands="${name}"]`).children].map(row => ({
      color: row.querySelector("select").value,
      threads: row.querySelector("input").value === "" ? null : Number(row.querySelector("input").value),
    })), $(`[data-plaid-repeat="${name}"]`).value === "mirrored");
    return { version: 1, palette, warp: axis("warp"),
      ...(!$("[data-plaid-linked]").checked ? { weft: axis("weft") } : {}),
      repeat_px: $("[data-plaid-size]").value === "" ? null : Number($("[data-plaid-size]").value),
      rotation_deg: $("[data-plaid-angle]").value === "" ? null : Number($("[data-plaid-angle]").value),
    };
  }

  function apply(value, controls) {
    model = value;
    $("[data-plaid-style]").textContent = value.css;
    $("[data-plaid-png]").src = value.png;
    $("[data-plaid-finish]").textContent = `${value.finish}. The preview includes the backing used for readable contrast.`;
    if (document.activeElement !== text) text.value = value.text;
    if (controls) renderControls();
    valid = true;
    saveState();
  }

  async function request(path, body, controls = false) {
    abort?.abort();
    abort = new AbortController();
    const requestId = requests.next();
    valid = false;
    saveState();
    try {
      const response = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal: abort.signal });
      const value = await response.json();
      if (!requests.isCurrent(requestId)) return;
      if (!response.ok) throw new Error(value.error || "The preview could not be loaded.");
      apply(value, controls);
      message("Draft preview. Use this plaid when you’re ready.");
      return value;
    } catch (error) {
      if (error.name !== "AbortError" && requests.isCurrent(requestId)) message(error.message || "The preview could not be loaded. Your changes are still here.");
    }
  }

  function schedule(spec, controls) {
    invalidate();
    message("Updating the preview…");
    timer = setTimeout(() => request("/admin/plaid/preview", { spec }, controls), 250);
  }

  function visualEdit() {
    if (busy) return;
    const spec = readControls();
    text.value = JSON.stringify(spec, null, 2);
    schedule(spec, false);
  }

  root.addEventListener("input", (event) => {
    if (busy) return;
    if (event.target === text) {
      invalidate();
      try { schedule(JSON.parse(text.value), true); }
      catch (error) { message(`Pattern text: ${error.message}`); }
    } else if (event.target !== seed && event.target.matches("input, select")) {
      if (event.target.matches("[data-plaid-linked]")) {
        $("[data-plaid-axis=weft]").hidden = event.target.checked;
        if (!event.target.checked) {
          const codes = $$("[data-plaid-palette] [data-code]").map(row => row.dataset.code);
          // cloneNode does not retain a select's live selected value. Build
          // from the visible controls so unlinking starts with identical yarns.
          $("[data-plaid-bands=weft]").replaceChildren(...[...$("[data-plaid-bands=warp]").children].map(row => bandRow({
            color: row.querySelector("select").value,
            threads: row.querySelector("input").value,
          }, codes)));
          $("[data-plaid-repeat=weft]").value = $("[data-plaid-repeat=warp]").value;
        }
      }
      visualEdit();
    }
  });

  root.addEventListener("click", async (event) => {
    const button = event.target.closest("button");
    if (!button || busy) return;
    if (button.hasAttribute("data-plaid-tab")) {
      for (const tab of $$("[data-plaid-tab]")) tab.setAttribute("aria-pressed", String(tab === button));
      for (const panel of $$("[data-plaid-panel]")) panel.hidden = panel.dataset.plaidPanel !== button.dataset.plaidTab;
      return;
    }
    if (button.hasAttribute("data-plaid-copy")) {
      try { await navigator.clipboard.writeText(text.value); message("Definition copied."); }
      catch { text.focus(); text.select(); message("Select and copy the definition above."); }
      return;
    }
    if (button.hasAttribute("data-plaid-reset")) { window.location.reload(); return; }
    if (button.hasAttribute("data-plaid-generate") || button.hasAttribute("data-plaid-repeat-seed")) {
      invalidate();
      let spec;
      try { spec = JSON.parse(text.value); } catch { message("Fix the pattern text before generating a variation."); return; }
      const repeat = button.hasAttribute("data-plaid-repeat-seed");
      const mode = repeat ? (lastGeneration?.mode || "all") : button.dataset.plaidGenerate;
      if (!repeat) seed.value = crypto.randomUUID();
      const retained = repeat ? (lastGeneration?.spec || spec) : spec;
      lastGeneration = { spec: structuredClone(retained), mode };
      message("Weaving a new draft…");
      await request("/admin/plaid/generate", { spec: retained, seed: seed.value, generator_version: generatorVersion, mode }, true);
      return;
    }
    if (button.hasAttribute("data-plaid-save")) {
      if (!valid) return;
      clearTimeout(timer);
      busy = true;
      message("Saving this plaid…");
      // Freeze editing until the receipt arrives; a save must never replace
      // characters entered while its request was in flight.
      for (const control of $$("input, select, textarea, button")) control.disabled = true;
      const result = await request("/admin/plaid", { spec: model.spec, expected_revision: revision }, true);
      busy = false;
      for (const control of $$("input, select, textarea, button")) control.disabled = false;
      if (result) {
        revision = result.revision;
        savedText = JSON.stringify(result.spec);
        const sheet = document.querySelector('link[href^="/plaid/current.css"]');
        if (sheet) sheet.href = `/plaid/current.css?revision=${revision}`;
        message("This is now the current plaid. Reloaded pages and Thursday workout cards will use it.");
      } else {
        // The last valid draft remains available for a save retry.
        valid = true;
      }
      saveState();
      return;
    }
    if (button.hasAttribute("data-plaid-add-color")) {
      const codes = $$("[data-plaid-palette] [data-code]").map(row => row.dataset.code);
      if (codes.length >= 16) { message("A plaid can have at most 16 colors."); return; }
      const code = [..."ABCDEFGHIJKLMNOPQRSTUVWXYZ"].find(code => !codes.includes(code));
      $("[data-plaid-palette]").append(colorRow(code, "#b7c6be"));
      for (const select of $$("[data-plaid-band-color]")) select.add(new Option(code, code));
    } else if (button.hasAttribute("data-plaid-remove-color")) {
      const row = button.closest("[data-code]");
      const code = row.dataset.code;
      if ($$("[data-plaid-palette] [data-code]").length <= 2) { message("Keep at least two palette colors."); return; }
      const activeBands = $("[data-plaid-linked]").checked ? $$("[data-plaid-bands=warp] [data-plaid-band-color]") : $$("[data-plaid-band-color]");
      if (activeBands.some(select => select.value === code)) { message(`Change the stripes using ${code} before removing that color.`); return; }
      row.remove();
      for (const select of $$("[data-plaid-band-color]")) [...select.options].find(option => option.value === code)?.remove();
    } else if (button.hasAttribute("data-plaid-add-band")) {
      const bands = $(`[data-plaid-bands="${button.dataset.plaidAddBand}"]`);
      if (bands.children.length >= 32) { message("An axis can have at most 32 stripes."); return; }
      const codes = $$("[data-plaid-palette] [data-code]").map(row => row.dataset.code);
      bands.append(bandRow({ color: codes[0], threads: 4 }, codes));
    } else if (button.hasAttribute("data-plaid-remove-band")) {
      const row = button.parentElement;
      if (row.parentElement.children.length <= 2) { message("Keep at least two stripes per axis."); return; }
      row.remove();
    } else if (button.hasAttribute("data-plaid-move")) {
      const row = button.parentElement;
      if (button.dataset.plaidMove === "-1" && row.previousElementSibling) row.previousElementSibling.before(row);
      if (button.dataset.plaidMove === "1" && row.nextElementSibling) row.nextElementSibling.after(row);
    } else return;
    visualEdit();
  });

  renderControls();
  saveState();
  return { readControls };
}

if (typeof document !== "undefined") {
  const root = document.querySelector("[data-plaid-editor]");
  if (root) mount(root);
}
