const root = document.querySelector("[data-fitness-entry]");

if (root) void start(root);

async function start(root) {
  const protocol = Number(root.dataset.entryProtocol);
  const statusNode = root.querySelector("[data-entry-status]");
  const context = { direction: "", query: "" };
  let registration;
  let snapshot;
  let activeLoadSetId = null;
  let publishInFlight = false;
  let rpcInFlight = 0;
  let refreshPending = false;
  let refreshInFlight = false;
  let requestTail = Promise.resolve();
  let searchGeneration = 0;

  const guide = parseGuide(root.dataset.entryGuide);
  const guideByName = new Map(guide.exercises.map((exercise) => [exercise.name, exercise]));
  const catalogNodes = new Map(
    Array.from(root.querySelectorAll("[data-exercise-catalog]")).map((node) => [
      node.dataset.name || "",
      node,
    ]),
  );

  function setStatus(kind, message) {
    if (!statusNode) return;
    statusNode.dataset.state = kind;
    statusNode.textContent = message;
  }

  try {
    requirePlatform();
    registration = await fitnessRegistration();
    await activeWorker(registration);
    snapshot = await request("bootstrap", {
      guide,
      now_utc: utcStamp(new Date()),
      context,
    });
    if (snapshot.legacy_storage_key) {
      try {
        localStorage.removeItem(snapshot.legacy_storage_key);
      } catch (_error) {
        // The new IndexedDB store is already durable; legacy cleanup is best effort.
      }
    }
    activeLoadSetId = snapshot.draft.exercises.at(-1)?.sets.at(-1)?.id || null;
    render(snapshot, { exercises: true });
    setStatus(
      "saved",
      snapshot.restored_start_reset
        ? "Restored your draft with a fresh start time."
        : "Draft saved on this device.",
    );
    installEventHandlers();
    updateTimer();
    window.setInterval(updateTimer, 1000);
    void kickFlush();
  } catch (error) {
    console.error("Workout entry failed to start", error);
    disableEntry();
    setStatus(
      "error",
      "Workout entry requires its Fitness worker, WebAssembly, and IndexedDB. Reload after running the current site build.",
    );
  }

  function installEventHandlers() {
    const review = root.querySelector("[data-entry-review]");
    review?.addEventListener("cancel", (event) => {
      if (publishInFlight) event.preventDefault();
    });

    root.addEventListener("click", (event) => {
      const control = event.target.closest("[data-action]");
      noteRowInteraction(event, control);
      if (!control || !root.contains(control)) return;
      const action = control.dataset.action;
      if (action === "open-receipt") return;
      event.preventDefault();
      void handleAction(control, action, event).catch(reportFailure);
    });

    root.addEventListener("input", (event) => {
      const input = event.target;
      if (input.matches("[data-entry-picker-search]")) {
        context.query = input.value;
        context.direction = context.query.trim() ? "" : context.direction;
        const generation = ++searchGeneration;
        void request("derive", { context }).then((value) => {
          if (generation !== searchGeneration) return;
          snapshot = value;
          renderGuidance(value);
          if (value.derived.search.length > 0) {
            requestAnimationFrame(scrollSearchIntoView);
          }
        }, reportFailure);
        return;
      }
      if (input.matches("[data-entry-review-title]")) {
        input.removeAttribute("aria-invalid");
        void mutate({ type: "set_title", value: input.value }, { exercises: false, quiet: true });
        return;
      }
      if (input.matches("[data-entry-review-notes]")) {
        input.removeAttribute("aria-invalid");
        void mutate({ type: "set_notes", value: input.value }, { exercises: false, quiet: true });
        return;
      }
      if (input.matches("[data-entry-field]")) {
        const field = input.dataset.entryField;
        if (field !== "weight" && field !== "reps" && field !== "effort") return;
        input.removeAttribute("aria-invalid");
        void mutate(
          {
            type: "set_field",
            exercise_id: input.dataset.exerciseId,
            set_id: input.dataset.setId,
            field,
            value: input.value,
          },
          { exercises: false, quiet: true },
        );
      }
    });

    root.addEventListener("change", (event) => {
      const input = event.target;
      if (!input.matches("[data-entry-field='setType']")) return;
      void mutate(
        {
          type: "set_type",
          exercise_id: input.dataset.exerciseId,
          set_id: input.dataset.setId,
          set_type: input.value,
        },
        { exercises: false },
      );
    });

    root.addEventListener("focusin", (event) => {
      const row = event.target.closest("[data-entry-set]");
      if (!row || !root.contains(row) || event.target.matches("[data-action='remove-set']")) return;
      activateLoadDock(row.dataset.setId);
    });
    root.addEventListener(
      "scroll",
      (event) => {
        if (event.target.matches?.(".entry-rir-options")) syncRirOverflow(event.target);
      },
      true,
    );
    root.addEventListener("keydown", (event) => {
      const search = root.querySelector("[data-entry-picker-search]");
      if (event.target !== search || event.key !== "Enter" || event.isComposing) return;
      const first = snapshot?.derived.search?.[0];
      if (!first) return;
      event.preventDefault();
      void addExercise(first.name);
    });

    navigator.serviceWorker.addEventListener("message", (event) => {
      if (
        event.data?.protocol === protocol &&
        event.data?.type === "fitness-entry-changed"
      ) {
        refreshPending = true;
        void drainRefresh();
      }
    });
    window.addEventListener("online", () => void kickFlush());
    window.addEventListener("pageshow", () => void kickFlush());
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") void kickFlush();
    });
  }

  async function handleAction(control, action, event) {
    if (action === "choose-direction") {
      context.direction = control.dataset.direction || "";
      context.query = "";
      const input = root.querySelector("[data-entry-picker-search]");
      if (input) input.value = "";
      snapshot = await request("derive", { context });
      renderGuidance(snapshot);
      return;
    }
    if (action === "add-exercise" || action === "use-suggestion" || action === "use-starter") {
      const name = control.dataset.name || control.dataset.suggestionName;
      if (name) await addExercise(name);
      return;
    }
    if (action === "remove-exercise") {
      const exercise = snapshot.draft.exercises.find(
        (candidate) => candidate.id === control.dataset.exerciseId,
      );
      if (!exercise || !window.confirm(`Remove ${exercise.name} from this workout?`)) return;
      await mutate({ type: "remove_exercise", exercise_id: exercise.id }, { exercises: true });
      return;
    }
    if (action === "add-set") {
      const setId = localId();
      activeLoadSetId = setId;
      await mutate(
        { type: "add_set", exercise_id: control.dataset.exerciseId, set_id: setId },
        { exercises: true },
      );
      return;
    }
    if (action === "remove-set") {
      const replacement = localId();
      await mutate(
        {
          type: "remove_set",
          exercise_id: control.dataset.exerciseId,
          set_id: control.dataset.setId,
          replacement_set_id: replacement,
        },
        { exercises: true },
      );
      return;
    }
    if (action === "toggle-set") {
      await mutate(
        {
          type: "toggle_set",
          exercise_id: control.dataset.exerciseId,
          set_id: control.dataset.setId,
        },
        { exercises: false },
      );
      return;
    }
    if (action === "use-load-preset") {
      await mutate(
        {
          type: "use_load",
          exercise_id: control.dataset.exerciseId,
          set_id: control.dataset.setId,
          weight_milli: control.dataset.weightMilli === "" ? null : Number(control.dataset.weightMilli),
          set_type: control.dataset.setType || null,
        },
        { exercises: true },
      );
      return;
    }
    if (action === "adjust-weight") {
      await mutate(
        {
          type: "adjust_weight",
          exercise_id: control.dataset.exerciseId,
          set_id: control.dataset.setId,
          delta_pounds: Number(control.dataset.weightDelta),
        },
        { exercises: true },
      );
      return;
    }
    if (action === "show-rir") {
      const row = control.closest("[data-entry-set]");
      activateLoadDock(row?.dataset.setId, { reveal: true });
      requestAnimationFrame(() => {
        revealSelectedRir(row);
        if (event.detail === 0) {
          (row?.querySelector("[data-entry-rir-option]:checked") ||
            row?.querySelector("[data-entry-rir-option]"))?.focus({ preventScroll: true });
        }
      });
      return;
    }
    if (action === "set-rir") {
      await mutate(
        {
          type: "set_rir",
          exercise_id: control.dataset.exerciseId,
          set_id: control.dataset.setId,
          effort_hundredths:
            control.dataset.effortHundredths === "" ? null : Number(control.dataset.effortHundredths),
          failure: control.dataset.failure === "true",
        },
        { exercises: true },
      );
      return;
    }
    if (action === "finish") {
      openReview();
      return;
    }
    if (action === "close-review") {
      if (!publishInFlight) closeReview();
      return;
    }
    if (action === "publish") {
      await publishWorkout();
      return;
    }
    if (action === "discard") {
      if (!window.confirm("Discard this entire workout draft?")) return;
      const value = await mutate(
        { type: "discard", now_utc: utcStamp(new Date()) },
        { exercises: true },
      );
      if (!value || value.error) return;
      closeReview();
      setStatus("saved", "Draft discarded. New workout started.");
      return;
    }
    if (action === "flush-queue") {
      await kickFlush();
      return;
    }
    if (action === "restore-failed") {
      const value = await request("restore", {
        queue_id: control.closest("[data-entry-queue-row]")?.dataset.queueId,
        now_utc: utcStamp(new Date()),
        context,
      });
      snapshot = value;
      render(value, { exercises: !value.error });
      if (value.error) {
        showActionError(value.error);
      } else {
        activeLoadSetId = value.draft.exercises.at(-1)?.sets.at(-1)?.id || null;
        render(value, { exercises: true });
        setStatus(
          "saved",
          value.restored_start_reset
            ? "Rejected workout restored with a fresh start time."
            : "Rejected workout restored as the current draft.",
        );
      }
      return;
    }
    if (action === "dismiss-receipt") {
      snapshot = await request("dismiss", {
        queue_id: control.closest("[data-entry-queue-row]")?.dataset.queueId,
        context,
      });
      renderQueue(snapshot);
      return;
    }
    if (action === "copy-receipt") {
      await copyReceipt(control.closest("[data-entry-queue-row]"));
    }
  }

  async function addExercise(name) {
    const exerciseId = localId();
    const setId = localId();
    activeLoadSetId = setId;
    context.query = "";
    const search = root.querySelector("[data-entry-picker-search]");
    if (search) search.value = "";
    await mutate(
      { type: "add_exercise", name, exercise_id: exerciseId, set_id: setId },
      { exercises: true },
    );
  }

  async function mutate(action, options) {
    try {
      const value = await request("transition", { action, context });
      snapshot = value;
      render(value, options);
      if (value.error) showActionError(value.error);
      else if (!options?.quiet) setStatus("saved", "Draft saved on this device.");
      applyEffect(value.effect);
      return value;
    } catch (error) {
      reportFailure(error);
      return null;
    }
  }

  function applyEffect(effect) {
    const focusExercise = effect?.focus_exercise;
    const focusSet = effect?.focus_set;
    if (focusExercise) {
      const exercise = snapshot.draft.exercises.find(
        (candidate) => candidate.id === focusExercise.exercise_id,
      );
      const set = exercise?.sets.at(-1);
      if (set) requestAnimationFrame(() => focusLoadDock(exercise.id, set.id));
    } else if (focusSet) {
      activeLoadSetId = focusSet.set_id;
      requestAnimationFrame(() => focusLoadDock(focusSet.exercise_id, focusSet.set_id));
    } else if (effect === "reset") {
      activeLoadSetId = snapshot.draft.exercises.at(-1)?.sets.at(-1)?.id || null;
    }
  }

  function openReview() {
    const review = root.querySelector("[data-entry-review]");
    const title = root.querySelector("[data-entry-review-title]");
    const notes = root.querySelector("[data-entry-review-notes]");
    if (title) title.value = snapshot.draft.title;
    if (notes) notes.value = snapshot.draft.notes;
    const omitted = root.querySelector("[data-entry-review-omitted]");
    if (omitted) {
      const count = snapshot.derived.unfinished_rows;
      omitted.hidden = count === 0;
      omitted.textContent = `${count} unfinished set ${count === 1 ? "row" : "rows"} will not be saved.`;
    }
    setReviewStatus("");
    if (!review?.open) {
      if (typeof review?.showModal === "function") review.showModal();
      else review?.setAttribute("open", "");
    }
  }

  function closeReview() {
    const review = root.querySelector("[data-entry-review]");
    if (typeof review?.close === "function") review.close();
    else review?.removeAttribute("open");
  }

  async function publishWorkout() {
    if (publishInFlight) return;
    setPublishLocked(true);
    setStatus("saving", "Publishing workout…");
    setReviewStatus("Publishing…");
    try {
      const value = await request("finalize", {
        ended_at_utc: utcStamp(new Date()),
        enqueued_at_ms: Date.now(),
        context,
      });
      snapshot = value;
      if (value.error) {
        render(value, { exercises: false });
        showActionError(value.error);
        setReviewStatus(value.error.message || "Review the workout fields.");
        return;
      }
      activeLoadSetId = null;
      context.direction = "";
      context.query = "";
      render(value, { exercises: true });
      closeReview();
      const queued = value.outbox.find((item) => item.queue_id === value.enqueued_queue_id);
      if (queued?.state === "saved") {
        setStatus("saved", queued.receipt?.duplicate ? "Workout already published." : "Workout published.");
      } else if (queued?.state === "failed") {
        setStatus("error", "The workout was rejected and kept for editing.");
      } else if (value.flush?.auth_blocked) {
        setStatus("offline", "Workout queued. Sign in again to publish it.");
      } else {
        setStatus("offline", "Workout queued on this device and will retry.");
      }
      requestAnimationFrame(() => {
        const row = queueRow(value.enqueued_queue_id);
        row?.scrollIntoView({ behavior: reducedMotion() ? "auto" : "smooth", block: "nearest" });
        row?.focus({ preventScroll: true });
      });
    } catch (error) {
      reportFailure(error);
      setReviewStatus(
        "The worker did not confirm finalization. Reload to recover the durable draft or queued workout.",
      );
    } finally {
      setPublishLocked(false);
    }
  }

  function setPublishLocked(locked) {
    publishInFlight = locked;
    const review = root.querySelector("[data-entry-review]");
    for (const control of review?.querySelectorAll("button, input, textarea, select") || []) {
      control.disabled = locked;
    }
  }

  function setReviewStatus(message) {
    const node = root.querySelector("[data-entry-review-status]");
    if (node) node.textContent = message;
  }

  function showActionError(error) {
    setStatus("error", error.message || "Review the workout fields.");
    if (error.review_field) {
      openReview();
      const input = root.querySelector(
        error.review_field === "title" ? "[data-entry-review-title]" : "[data-entry-review-notes]",
      );
      input?.setAttribute("aria-invalid", "true");
      input?.focus();
      setReviewStatus(error.message || "Review the workout fields.");
      return;
    }
    if (error.exercise_id && error.set_id && error.field) {
      const input = Array.from(root.querySelectorAll("[data-entry-field]")).find(
        (candidate) =>
          candidate.dataset.exerciseId === error.exercise_id &&
          candidate.dataset.setId === error.set_id &&
          candidate.dataset.entryField === error.field,
      );
      input?.setAttribute("aria-invalid", "true");
      input?.focus();
    }
  }

  async function kickFlush() {
    if (!snapshot) return;
    try {
      const value = await request("flush", { context });
      snapshot = value;
      render(value, { exercises: false });
      if (value.flush?.auth_blocked) {
        setStatus("offline", "Queued workouts are waiting for you to sign in again.");
      }
    } catch (error) {
      reportFailure(error);
    }
  }

  async function refresh() {
    try {
      const value = await request("snapshot", { context });
      snapshot = value;
      render(value, { exercises: true });
    } catch (error) {
      reportFailure(error);
    }
  }

  async function drainRefresh() {
    if (!refreshPending || refreshInFlight || rpcInFlight > 0) return;
    refreshPending = false;
    refreshInFlight = true;
    try {
      await refresh();
    } finally {
      refreshInFlight = false;
      if (refreshPending) void drainRefresh();
    }
  }

  function render(value, { exercises = false } = {}) {
    if (exercises) renderExercises(value);
    else syncExistingRows(value);
    renderGuidance(value);
    renderQueue(value);
    updateTimer();
  }

  function renderExercises(value) {
    const exerciseTemplate = root.querySelector("[data-entry-exercise-template]");
    const setTemplate = root.querySelector("[data-entry-set-template]");
    const exercisesNode = root.querySelector("[data-entry-exercises]");
    if (!exerciseTemplate || !setTemplate || !exercisesNode) return;
    const setViews = new Map(value.derived.set_views.map((view) => [view.id, view]));
    exercisesNode.replaceChildren();

    for (const exercise of value.draft.exercises) {
      const guide = guideByName.get(exercise.name) || {
        bodyweight: false,
        marks: [],
        loads: [],
      };
      const card = exerciseTemplate.content.firstElementChild.cloneNode(true);
      card.dataset.exerciseId = exercise.id;
      card.querySelector("[data-entry-exercise-name]").textContent = exercise.name;
      for (const action of card.querySelectorAll("[data-exercise-action]")) {
        action.dataset.exerciseId = exercise.id;
      }
      const marks = card.querySelectorAll("[data-entry-mark]");
      const marksNode = card.querySelector("[data-entry-prs]");
      if (marksNode) marksNode.hidden = guide.marks.length === 0;
      marks.forEach((node, index) => {
        const mark = guide.marks[index];
        node.hidden = !mark;
        if (!mark) return;
        node.querySelector("[data-entry-mark-kind]").textContent = mark.kind;
        node.querySelector("[data-entry-mark-value]").textContent = mark.value;
        node.querySelector("[data-entry-mark-detail]").textContent = mark.detail || "";
        node.toggleAttribute("data-primary", index === 0);
      });

      const list = card.querySelector("[data-entry-set-list]");
      exercise.sets.forEach((set, index) => {
        const setView = setViews.get(set.id);
        if (!setView) return;
        const row = setTemplate.content.firstElementChild.cloneNode(true);
        row.dataset.setId = set.id;
        row.dataset.exerciseId = exercise.id;
        row.dataset.setNumber = String(index + 1);
        row.dataset.complete = String(set.done);
        row.querySelector("[data-entry-set-number]").textContent = String(index + 1);
        for (const action of row.querySelectorAll("[data-set-action]")) {
          action.dataset.exerciseId = exercise.id;
          action.dataset.setId = set.id;
        }
        for (const input of row.querySelectorAll("[data-entry-field]")) {
          input.dataset.exerciseId = exercise.id;
          input.dataset.setId = set.id;
          if (input.dataset.entryField === "setType") input.value = set.set_type;
          if (input.dataset.entryField === "weight") {
            input.value = set.weight;
            input.placeholder = guide.bodyweight ? "BW" : "—";
            const label = input.closest("label")?.querySelector("[data-entry-weight-label]");
            if (label) {
              label.textContent = guide.bodyweight
                ? "Added weight or assistance in pounds; leave blank for bodyweight"
                : "Weight in pounds; optional";
            }
          }
          if (input.dataset.entryField === "reps") input.value = set.reps;
        }
        renderLoadPresets(row, guide, set, setView, index + 1);
        syncSetRow(row, set, setView, index + 1);
        const dock = row.querySelector("[data-entry-load-dock]");
        if (dock) {
          dock.id = `entry-set-tools-${set.id}`;
          dock.hidden = set.id !== activeLoadSetId;
        }
        list.append(row);
      });
      exercisesNode.append(card);
    }
    requestAnimationFrame(() => revealSelectedRir(findSetRow(activeLoadSetId)));
  }

  function renderLoadPresets(row, guide, set, setView, setNumber) {
    const buttons = row.querySelectorAll("[data-entry-load-preset]");
    let visible = 0;
    buttons.forEach((button, index) => {
      const preset = guide.loads[index];
      button.hidden = !preset;
      if (!preset) return;
      visible += 1;
      button.dataset.weightMilli = preset.weight_milli === null ? "" : String(preset.weight_milli);
      button.dataset.setType = preset.set_type;
      button.querySelector("[data-entry-load-preset-kind]").textContent = preset.label.toUpperCase();
      button.querySelector("[data-entry-load-preset-value]").textContent = preset.display;
      button.setAttribute(
        "aria-label",
        `Use ${preset.spoken} as ${preset.set_type === "WARMUP_SET" ? "warm-up" : "working"} set ${setNumber}`,
      );
      const selectedWeight = set.weight === "" ? null : setView.weight_milli;
      button.setAttribute(
        "aria-pressed",
        String(
          setView.weight_valid &&
            selectedWeight === preset.weight_milli &&
            set.set_type === preset.set_type,
        ),
      );
    });
    const rail = row.querySelector("[data-entry-load-presets]");
    if (rail) rail.hidden = visible === 0;
    for (const step of row.querySelectorAll("[data-action='adjust-weight']")) {
      const amount = Number(step.dataset.weightDelta);
      step.setAttribute(
        "aria-label",
        `${amount < 0 ? "Subtract" : "Add"} ${Math.abs(amount)} pounds ${amount < 0 ? "from" : "to"} set ${setNumber}`,
      );
    }
  }

  function syncExistingRows(value) {
    const views = new Map(value.derived.set_views.map((view) => [view.id, view]));
    for (const exercise of value.draft.exercises) {
      exercise.sets.forEach((set, index) => {
        const row = findSetRow(set.id);
        const view = views.get(set.id);
        if (row && view) syncSetRow(row, set, view, index + 1);
      });
    }
  }

  function syncSetRow(row, set, view, setNumber) {
    row.dataset.complete = String(set.done);
    row.setAttribute("aria-label", `Set ${setNumber}, ${view.set_type_spoken}`);
    const ordinal = row.querySelector("[data-entry-set-ordinal]");
    if (ordinal) ordinal.dataset.kind = view.set_kind;
    row.querySelector("[data-entry-set-type]").textContent = view.set_type_label;
    const select = row.querySelector("[data-entry-field='setType']");
    if (select) {
      select.value = set.set_type;
      select.setAttribute("aria-label", `Set ${setNumber} type`);
    }
    const done = row.querySelector("[data-action='toggle-set']");
    done?.setAttribute("aria-pressed", String(set.done));
    done?.setAttribute("aria-label", set.done ? "Mark set incomplete" : "Mark set complete");
    const rir = row.querySelector("[data-entry-field='effort']");
    const dock = row.querySelector("[data-entry-load-dock]");
    if (rir) {
      rir.querySelector("[data-entry-rir-value]").textContent = view.rir_display;
      rir.setAttribute("aria-label", `Set ${setNumber} reps in reserve, ${view.rir_spoken}`);
      rir.setAttribute("aria-expanded", String(Boolean(dock && !dock.hidden)));
      if (dock?.id) rir.setAttribute("aria-controls", dock.id);
      rir.toggleAttribute("aria-invalid", !view.effort_valid);
    }
    row.querySelector("[data-entry-rir-picker]")?.setAttribute(
      "aria-label",
      `Reps in reserve for set ${setNumber}`,
    );
    for (const option of row.querySelectorAll("[data-entry-rir-option]")) {
      const raw = option.dataset.effortHundredths;
      const effort = raw === "" ? null : Number(raw);
      const failure = option.dataset.failure === "true";
      const selected = failure
        ? view.failure
        : !view.failure && (set.effort === "" ? effort === null : view.effort_hundredths === effort);
      option.name = `entry-rir-${set.id}`;
      option.checked = selected;
      option.setAttribute(
        "aria-label",
        failure
          ? `Failure, set ${setNumber}`
          : effort === null
          ? `Do not record reps in reserve for set ${setNumber}`
          : `${1000 - effort === 50 ? "Half a" : String((1000 - effort) / 100)} reps in reserve, set ${setNumber}`,
      );
    }
  }

  function renderGuidance(value) {
    setText("[data-entry-exercise-count]", value.derived.exercise_count);
    setText("[data-entry-set-count]", value.derived.set_count);
    setText("[data-entry-completed-count]", value.derived.completed_count);
    for (const button of root.querySelectorAll("[data-entry-finish]")) {
      button.disabled = !value.derived.finish_enabled;
    }
    const coverageNodes = root.querySelectorAll("[data-entry-coverage-item]");
    coverageNodes.forEach((node, index) => {
      const coverage = value.derived.coverage[index];
      node.hidden = !coverage;
      if (!coverage) return;
      node.querySelector("[data-entry-coverage-label]").textContent = coverage.label;
      node.querySelector("[data-entry-coverage-value]").textContent = coverage.level;
    });

    const directions = root.querySelector("[data-entry-directions]");
    if (directions) directions.hidden = value.derived.has_active_exercise || context.query.trim() !== "";
    for (const button of directions?.querySelectorAll("[data-direction]") || []) {
      const active = button.dataset.direction === context.direction;
      button.setAttribute("aria-pressed", String(active));
      button.toggleAttribute("data-active", active);
    }
    const starters = root.querySelector("[data-entry-starters]");
    const starterButtons = root.querySelectorAll("[data-entry-starter]");
    starterButtons.forEach((button, index) => {
      const suggestion = value.derived.starters[index];
      button.hidden = !suggestion;
      if (!suggestion) {
        button.removeAttribute("data-suggestion-name");
        return;
      }
      button.dataset.lane = suggestion.lane;
      button.dataset.suggestionName = suggestion.name;
      button.querySelector("[data-entry-starter-lane]").textContent = suggestion.label;
      button.querySelector("[data-entry-starter-name]").textContent = suggestion.name;
      button.querySelector("[data-entry-starter-mark]").textContent = suggestion.mark;
      button.setAttribute("aria-label", suggestion.aria_label);
    });
    if (starters) starters.hidden = value.derived.starters.length === 0 || context.query.trim() !== "";

    const fork = root.querySelector("[data-entry-fork]");
    if (fork) fork.hidden = !value.derived.has_active_exercise;
    renderLane("deepen", value.derived.deepen);
    renderLane("expand", value.derived.expand);

    for (const node of catalogNodes.values()) node.hidden = true;
    const results = root.querySelector("[data-entry-search-results]");
    for (const hit of value.derived.search) {
      const node = catalogNodes.get(hit.name);
      if (!node) continue;
      node.hidden = false;
      results?.append(node);
    }
    if (results) results.hidden = context.query.trim() === "" || value.derived.search.length === 0;
    const search = root.querySelector("[data-entry-picker-search]");
    search?.setAttribute(
      "aria-expanded",
      String(context.query.trim() !== "" && value.derived.search.length > 0),
    );
    setText("[data-entry-search-feedback]", value.derived.search_feedback);
    const empty = root.querySelector("[data-entry-quick-empty]");
    if (empty) {
      empty.textContent = value.derived.quick_empty;
      empty.hidden = value.derived.quick_empty === "";
    }
  }

  function renderLane(lane, suggestion) {
    const node = Array.from(root.querySelectorAll("[data-lane]")).find(
      (candidate) => candidate.dataset.lane === lane,
    );
    if (!node) return;
    const action = node.querySelector("[data-action='use-suggestion']");
    if (!suggestion) {
      action.disabled = true;
      action.removeAttribute("data-suggestion-name");
      action.setAttribute("aria-label", "No suggestion available");
      node.querySelector("[data-entry-suggestion-choice]").textContent = "No route yet";
      node.querySelector("[data-entry-suggestion-reason]").textContent = "Add an exercise to open this branch.";
      node.querySelector("[data-entry-suggestion-mark]").textContent = "";
      return;
    }
    action.disabled = false;
    action.dataset.suggestionName = suggestion.name;
    action.setAttribute("aria-label", suggestion.aria_label);
    node.querySelector("[data-entry-suggestion-choice]").textContent = suggestion.name;
    node.querySelector("[data-entry-suggestion-reason]").textContent = suggestion.reason;
    node.querySelector("[data-entry-suggestion-mark]").textContent = suggestion.mark;
  }

  function scrollSearchIntoView() {
    const scroller = root.querySelector(".entry-scroll");
    const search = root.querySelector("[data-entry-picker-search]");
    if (!scroller || !search || context.query.trim() === "") return;
    const scrollerRect = scroller.getBoundingClientRect();
    const searchRect = search.getBoundingClientRect();
    const target = scroller.scrollTop + searchRect.top - scrollerRect.top;
    scroller.scrollTo({ top: Math.max(0, target), behavior: "auto" });
  }

  function renderQueue(value) {
    const section = root.querySelector("[data-entry-queue]");
    const list = root.querySelector("[data-entry-queue-list]");
    if (!section || !list) return;
    list.replaceChildren();
    const rows = [...value.outbox].reverse();
    for (const queued of rows) {
      const selector = `[data-entry-${queued.state}-template]`;
      const template = root.querySelector(selector);
      if (!template) continue;
      const row = template.content.firstElementChild.cloneNode(true);
      row.dataset.queueId = queued.queue_id;
      row.querySelector("[data-entry-queue-title]").textContent = queued.workout.title;
      if (queued.state === "pending") {
        const predicted = row.querySelector("[data-entry-predicted-location]");
        const wrap = row.querySelector("[data-entry-predicted-wrap]");
        if (predicted && queued.predicted_location) {
          predicted.textContent = new URL(queued.predicted_location, location.origin).href;
        } else if (wrap) {
          wrap.hidden = true;
        }
      } else if (queued.state === "failed") {
        row.querySelector("[data-entry-failure]").textContent =
          queued.failure || "The server rejected this workout.";
      } else if (queued.state === "saved") {
        const receipt = queued.receipt;
        row.querySelector("[data-entry-share-text]").value = receipt?.share_text || "";
        const open = row.querySelector("[data-action='open-receipt']");
        const location = trustedWorkoutLocation(receipt?.location);
        if (location) open.href = location;
        else open.hidden = true;
      }
      list.append(row);
    }
    section.hidden = rows.length === 0;
  }

  async function copyReceipt(row) {
    const text = row?.querySelector("[data-entry-share-text]")?.value;
    const status = row?.querySelector("[data-entry-copy-status]");
    if (!text || !navigator.clipboard?.writeText) {
      if (status) status.textContent = "Clipboard access is unavailable; select the text above.";
      return;
    }
    try {
      await navigator.clipboard.writeText(text);
      if (status) status.textContent = "Copied canonical share text.";
    } catch (_error) {
      if (status) status.textContent = "Copy failed; select the text above.";
    }
  }

  function noteRowInteraction(event, control) {
    const row = event.target.closest("[data-entry-set]");
    if (!row || !root.contains(row) || control?.dataset.action === "remove-set") return;
    const textEntry = Boolean(event.target.closest("input, select, textarea"));
    activateLoadDock(row.dataset.setId, { reveal: !textEntry });
  }

  function activateLoadDock(setId, { reveal = false } = {}) {
    if (!setId) return;
    activeLoadSetId = setId;
    for (const row of root.querySelectorAll("[data-entry-set]")) {
      const active = row.dataset.setId === setId;
      const dock = row.querySelector("[data-entry-load-dock]");
      if (dock) dock.hidden = !active;
      row.querySelector("[data-action='show-rir']")?.setAttribute("aria-expanded", String(active));
    }
    if (reveal) {
      requestAnimationFrame(() =>
        findSetRow(setId)?.scrollIntoView({
          behavior: reducedMotion() ? "auto" : "smooth",
          block: "nearest",
        }),
      );
    }
  }

  function focusLoadDock(exerciseId, setId) {
    const row = Array.from(root.querySelectorAll("[data-entry-set]")).find(
      (candidate) =>
        candidate.dataset.exerciseId === exerciseId && candidate.dataset.setId === setId,
    );
    if (!row) return;
    activateLoadDock(setId);
    row.scrollIntoView({ behavior: reducedMotion() ? "auto" : "smooth", block: "center" });
    (row.querySelector("[data-entry-load-preset]:not([hidden])") ||
      row.querySelector("[data-entry-load-dock]"))?.focus({ preventScroll: true });
  }

  function revealSelectedRir(row) {
    const rail = row?.querySelector(".entry-rir-options");
    const selected = row?.querySelector("[data-entry-rir-option]:checked")?.closest(".entry-rir-option");
    if (!rail || !selected) {
      if (rail) syncRirOverflow(rail);
      return;
    }
    const railRect = rail.getBoundingClientRect();
    const selectedRect = selected.getBoundingClientRect();
    let left = rail.scrollLeft;
    if (selectedRect.left < railRect.left) left -= railRect.left - selectedRect.left;
    else if (selectedRect.right > railRect.right) left += selectedRect.right - railRect.right;
    left = Math.min(Math.max(0, rail.scrollWidth - rail.clientWidth), Math.max(0, left));
    if (Math.abs(left - rail.scrollLeft) > 1) rail.scrollTo({ left, behavior: "auto" });
    syncRirOverflow(rail);
  }

  function syncRirOverflow(rail) {
    const picker = rail.closest("[data-entry-rir-picker]");
    if (!picker) return;
    const maximum = Math.max(0, rail.scrollWidth - rail.clientWidth);
    picker.toggleAttribute("data-overflow-left", rail.scrollLeft > 1);
    picker.toggleAttribute("data-overflow-right", rail.scrollLeft < maximum - 1);
  }

  function updateTimer() {
    if (!snapshot) return;
    const started = Date.parse(`${snapshot.draft.started_at_utc.replace(" ", "T")}Z`);
    const seconds = Number.isFinite(started) ? Math.max(0, Math.floor((Date.now() - started) / 1000)) : 0;
    const value = formatElapsed(seconds);
    for (const node of root.querySelectorAll("[data-entry-timer]")) {
      node.textContent = value;
      node.setAttribute("datetime", `PT${seconds}S`);
    }
  }

  function setText(selector, value) {
    const node = root.querySelector(selector);
    if (node) node.textContent = String(value);
  }

  function findSetRow(setId) {
    return Array.from(root.querySelectorAll("[data-entry-set]")).find(
      (row) => row.dataset.setId === setId,
    );
  }

  function queueRow(queueId) {
    return Array.from(root.querySelectorAll("[data-entry-queue-row]")).find(
      (row) => row.dataset.queueId === queueId,
    );
  }

  function reportFailure(error) {
    console.error("Fitness entry worker request failed", error);
    setStatus("error", error instanceof Error ? error.message : "Fitness entry worker failed.");
  }

  function request(method, payload) {
    const call = requestTail.then(
      () => sendRequest(method, payload),
      () => sendRequest(method, payload),
    );
    requestTail = call.catch(() => {});
    return call;
  }

  async function sendRequest(method, payload) {
    const worker = await activeWorker(registration);
    const requestId = localId();
    const channel = new MessageChannel();
    rpcInFlight += 1;
    try {
      return await new Promise((resolve, reject) => {
        const timer = window.setTimeout(() => {
          channel.port1.close();
          reject(new Error("Fitness entry worker did not respond."));
        }, 30_000);
        channel.port1.onmessage = (event) => {
          window.clearTimeout(timer);
          const reply = event.data;
          if (reply?.protocol !== protocol || reply?.request_id !== requestId) {
            reject(new Error("Fitness entry worker returned a mismatched reply."));
          } else if (!reply.ok) {
            reject(new Error(reply.error || "Fitness entry worker failed."));
          } else {
            resolve(reply.value);
          }
          channel.port1.close();
        };
        try {
          worker.postMessage(
            { protocol, request_id: requestId, method, payload },
            [channel.port2],
          );
        } catch (error) {
          window.clearTimeout(timer);
          channel.port1.close();
          reject(error);
        }
      });
    } finally {
      rpcInFlight -= 1;
      if (rpcInFlight === 0 && refreshPending) void drainRefresh();
    }
  }
}

function parseGuide(raw) {
  let guide;
  try {
    guide = JSON.parse(raw || "");
  } catch (_error) {
    throw new Error("The server returned malformed Fitness guide data.");
  }
  if (!guide || !Array.isArray(guide.exercises)) {
    throw new Error("The server returned incomplete Fitness guide data.");
  }
  return guide;
}

function requirePlatform() {
  if (
    !("serviceWorker" in navigator) ||
    !("indexedDB" in globalThis) ||
    !("WebAssembly" in globalThis) ||
    !("MessageChannel" in globalThis)
  ) {
    throw new Error("This browser does not provide the required local-first APIs.");
  }
}

async function fitnessRegistration() {
  const registration = await (
    globalThis.FITNESS_SERVICE_WORKER ||
    navigator.serviceWorker.register("/fitness/sw.js", { scope: "/fitness" })
  );
  await registration.update();
  return registration;
}

async function activeWorker(registration) {
  const candidate = registration.installing || registration.waiting;
  if (!candidate && registration.active) return registration.active;
  if (!candidate) {
    await registration.update();
    return activeWorker(registration);
  }
  if (candidate.state === "activated") return candidate;
  await new Promise((resolve, reject) => {
    const timer = window.setTimeout(
      () => reject(new Error("Fitness entry worker activation timed out.")),
      30_000,
    );
    candidate.addEventListener("statechange", () => {
      if (candidate.state === "activated") {
        window.clearTimeout(timer);
        resolve();
      } else if (candidate.state === "redundant") {
        window.clearTimeout(timer);
        reject(new Error("Fitness entry worker could not activate."));
      }
    });
  });
  return candidate;
}

function localId() {
  const value = globalThis.crypto?.randomUUID?.();
  if (!value) throw new Error("Secure local identities are unavailable.");
  return value;
}

function utcStamp(date) {
  return date.toISOString().slice(0, 19).replace("T", " ");
}

function formatElapsed(seconds) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(remainder).padStart(2, "0")}`
    : `${minutes}:${String(remainder).padStart(2, "0")}`;
}

function trustedWorkoutLocation(raw) {
  if (typeof raw !== "string") return null;
  try {
    const url = new URL(raw, location.origin);
    if (url.origin !== location.origin || !url.pathname.startsWith("/fitness/lift/")) return null;
    return `${url.pathname}${url.search}${url.hash}`;
  } catch (_error) {
    return null;
  }
}

function reducedMotion() {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function disableEntry() {
  root?.setAttribute("data-entry-unavailable", "");
  for (const control of root?.querySelectorAll("button, input, textarea, select") || []) {
    control.disabled = true;
  }
}
