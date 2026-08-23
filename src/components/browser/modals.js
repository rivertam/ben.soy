// The shared overlay driver. Event delegation over the whole document turns
// native dialogs and inline citation popovers into working controls. Nothing
// here touches class names — Tailwind scans .rs only, and behavior rides on
// data attributes and native element state.
//
// Dialog contract:
//   <dialog data-modal id="…">          the surface (components::modal renders it)
//     … [data-modal-close] …            any control that dismisses it
//   [data-modal-open-on-load]           open as soon as parsed (errors/notices)
//   <a data-modal-open="id" href="…">   a trigger anywhere; the href is the
//                                        no-JS / unsupported-browser fallback
// Dialogs also emit bubbling `modal:open` / `modal:close` events, the seam a
// feature-specific companion (e.g. auth-dialog.js) hooks without re-deriving
// any of the above.
//
// Inline-popover contract:
//   <a data-inline-popover-trigger="id"> a genuinely inline, wrapping trigger
//   <span id="id" popover>                the server-rendered popover
//   [data-dont-obstruct]                  a hard exclusion for rail cards/traces
//   [data-inline-popover-rail-side]       optional left/right art direction
//   [data-inline-popover-slot="name"]     optional free-placement marker
// Native `popovertarget` only works on atomic form controls. The small driver
// lets prose use a fragmenting anchor while retaining native light-dismiss,
// Escape, and top-layer behavior on the popover itself.

// Which element to refocus when a given dialog closes (its opener).
const openers = new WeakMap();

const canPopover = (popover) =>
  popover instanceof HTMLElement &&
  typeof popover.showPopover === "function" &&
  typeof popover.hidePopover === "function";

const popoverFor = (trigger) =>
  document.getElementById(trigger.getAttribute("data-inline-popover-trigger"));

const setPopoverExpanded = (popover) => {
  const expanded = popover.matches(":popover-open") ? "true" : "false";
  for (const trigger of document.querySelectorAll(
    `[data-inline-popover-trigger="${CSS.escape(popover.id)}"]`,
  )) {
    trigger.setAttribute("aria-expanded", expanded);
  }
};

// On a wide, precise-pointer viewport, inline notes become a live annotation
// rail. The layout is deliberately geometry-driven: prose can use the shared
// component anywhere without declaring a rail, a side, or a vertical slot.
// Cards only enter a gutter that is actually wide enough. A small label
// solver then chooses a sparse, stable constellation before positioning it;
// this keeps the rails from turning into two tightly packed footnote stacks.
// Narrow/coarse-pointer layouts retain the ordinary native-popover behavior.
const createInlinePopoverRails = () => {
  const MIN_RAIL_WIDTH = 232;
  const MIN_SLOT_WIDTH = 180;
  const MAX_RAIL_WIDTH = 300;
  const EDGE_INSET = 16;
  const SOURCE_GAP = 22;
  const LEFT_SOURCE_PORT_GAP = 10;
  const RIGHT_SOURCE_PORT_GAP = 6;
  const OBSTRUCTION_GAP = 12;
  const TRACE_OBSTRUCTION_GAP = 2;
  const MIN_CARD_GAP = 64;
  const MAX_AUTHORED_SLOT_DRIFT = 48;
  const SAFE_TOP = 18;
  // Fixed chrome participates in the same obstruction map as authored page
  // content, so cards can use the lower viewport when that horizontal lane
  // is genuinely clear.
  const SAFE_BOTTOM = 18;
  const MAX_CANDIDATES = 8;
  const MAX_VISIBLE = 3;
  const TARGET_VERTICAL_SLOT = 380;
  const HOVER_GRACE_MS = 320;
  const REVEAL_SCROLL_Y = 100;
  const desktop = window.matchMedia(
    "(min-width: 64rem) and (hover: hover) and (pointer: fine)",
  );
  const revealScrollY = () => {
    const maxScroll = Math.max(
      0,
      document.documentElement.scrollHeight -
        document.documentElement.clientHeight,
    );
    // Preserve the deliberate 100px pause on normal pages, but don't make
    // the rail impossible to reveal on a page whose entire scroll is shorter.
    // If all of the page is already visible, its constellation can appear
    // immediately because there is no scroll gesture to wait for.
    return Math.min(REVEAL_SCROLL_Y, maxScroll);
  };
  const railsRevealed = () => window.scrollY >= revealScrollY();
  const svgNamespace = "http://www.w3.org/2000/svg";
  const makeLineLayer = (depth) => {
    const root = document.createElementNS(svgNamespace, "svg");
    root.setAttribute("data-inline-popover-lines", depth);
    root.setAttribute("aria-hidden", "true");

    const maskId = `inline-popover-obstruction-mask-${depth}`;
    const definitions = document.createElementNS(svgNamespace, "defs");
    const mask = document.createElementNS(svgNamespace, "mask");
    mask.setAttribute("id", maskId);
    mask.setAttribute("maskUnits", "userSpaceOnUse");
    mask.setAttribute("maskContentUnits", "userSpaceOnUse");
    const maskField = document.createElementNS(svgNamespace, "rect");
    maskField.setAttribute("fill", "white");
    mask.append(maskField);
    definitions.append(mask);

    const traces = document.createElementNS(svgNamespace, "g");
    traces.setAttribute("mask", `url(#${maskId})`);
    root.append(definitions, traces);
    return { root, mask, maskField, traces };
  };
  const ambientLineLayer = makeLineLayer("ambient");
  const focusedLineLayer = makeLineLayer("focused");
  const lineLayers = [ambientLineLayer, focusedLineLayer];

  const placementSlots = new Map();
  for (const slot of document.querySelectorAll("[data-inline-popover-slot]")) {
    const name = slot.getAttribute("data-inline-popover-slot");
    if (name && !placementSlots.has(name)) placementSlots.set(name, slot);
  }
  const seenPanels = new Set();
  const entries = [];
  for (const trigger of document.querySelectorAll(
    "[data-inline-popover-trigger]",
  )) {
    const panel = popoverFor(trigger);
    if (
      !canPopover(panel) ||
      !panel.hasAttribute("data-inline-popover-panel") ||
      seenPanels.has(panel)
    ) {
      continue;
    }
    seenPanels.add(panel);

    const path = document.createElementNS(svgNamespace, "path");
    path.setAttribute("data-inline-popover-line", "");
    const port = document.createElementNS(svgNamespace, "circle");
    port.setAttribute("data-inline-popover-port", "");
    port.setAttribute("r", "2.5");
    ambientLineLayer.traces.append(path, port);

    const kicker = panel.querySelector("[data-inline-popover-kicker]");
    kicker?.setAttribute(
      "data-inline-popover-channel",
      String(entries.length + 1).padStart(2, "0"),
    );
    entries.push({
      trigger,
      panel,
      path,
      port,
      preferredSide: ["left", "right"].includes(
        trigger.getAttribute("data-inline-popover-rail-side"),
      )
        ? trigger.getAttribute("data-inline-popover-rail-side")
        : null,
      preferredSlot: placementSlots.get(
        trigger.getAttribute("data-inline-popover-rail-slot"),
      ),
      rail: false,
      previousSide: null,
      geometry: null,
    });
  }

  if (!entries.length) {
    return {
      handles: () => false,
      toggle: () => {},
      dismiss: () => false,
      dismissFocused: () => false,
    };
  }

  document.body.append(...lineLayers.map((layer) => layer.root));
  const entryForPanel = new Map(entries.map((entry) => [entry.panel, entry]));
  const dismissed = new Set();
  let animationFrame = 0;
  let forced = null;
  let hovered = null;
  let panelFocused = null;
  let hoverReleaseTimer = 0;
  let focused = null;
  // The quiet opening state is one-shot. Reading far enough or deliberately
  // touching an annotation latches the live constellation on for this visit.
  let readerConsented = false;

  const interactionEntry = () => hovered || panelFocused || forced;
  const railsAreActivated = () => readerConsented || railsRevealed();

  const clearTrace = (entry) => {
    entry.path.removeAttribute("d");
    entry.path.removeAttribute("data-inline-popover-focused");
    entry.port.removeAttribute("cx");
    entry.port.removeAttribute("cy");
    entry.port.removeAttribute("data-inline-popover-focused");
    entry.trigger.removeAttribute("data-inline-popover-current");
    if (entry.path.parentNode !== ambientLineLayer.traces) {
      ambientLineLayer.traces.append(entry.path, entry.port);
    }
  };

  const resetRail = (entry) => {
    clearTrace(entry);
    entry.panel.removeAttribute("data-inline-popover-focused");
    if (!entry.rail) return;
    if (entry.panel.matches(":popover-open")) entry.panel.hidePopover();
    entry.panel.setAttribute("popover", "auto");
    entry.panel.setAttribute("role", "dialog");
    entry.panel.removeAttribute("data-inline-popover-rail-active");
    entry.panel.removeAttribute("data-inline-popover-side");
    entry.panel.style.removeProperty("--inline-popover-rail-x");
    entry.panel.style.removeProperty("--inline-popover-rail-y");
    entry.panel.style.removeProperty("--inline-popover-rail-width");
    entry.rail = false;
    entry.geometry = null;
  };

  const deactivate = () => {
    for (const entry of entries) resetRail(entry);
    document.documentElement.removeAttribute("data-inline-popover-rails");
    dismissed.clear();
    forced = null;
    hovered = null;
    panelFocused = null;
    window.clearTimeout(hoverReleaseTimer);
    hoverReleaseTimer = 0;
    focused = null;
  };

  const hostFor = (trigger) =>
    trigger.closest(
      "p, li, blockquote, figcaption, dd, dt, h1, h2, h3, h4, td, th, label",
    ) || trigger.parentElement;

  const rectanglesOverlap = (first, second, gap = 0) =>
    first.left < second.right + gap &&
    first.right > second.left - gap &&
    first.top < second.bottom + gap &&
    first.bottom > second.top - gap;

  const obstructionRects = (viewportWidth, viewportHeight) =>
    [...document.querySelectorAll("[data-dont-obstruct]")].flatMap(
      (element) =>
        [...element.getClientRects()]
          .filter(
            (rect) =>
              rect.width > 0 &&
              rect.height > 0 &&
              rect.right > 0 &&
              rect.bottom > 0 &&
              rect.left < viewportWidth &&
              rect.top < viewportHeight,
          )
          .map((rect) => ({
            left: Math.max(0, rect.left),
            right: Math.min(viewportWidth, rect.right),
            top: Math.max(0, rect.top),
            bottom: Math.min(viewportHeight, rect.bottom),
          })),
    );

  const updateLineMasks = (obstructions, viewportWidth, viewportHeight) => {
    for (const layer of lineLayers) {
      layer.root.setAttribute(
        "viewBox",
        `0 0 ${viewportWidth} ${viewportHeight}`,
      );
      layer.mask.setAttribute("x", "0");
      layer.mask.setAttribute("y", "0");
      layer.mask.setAttribute("width", String(viewportWidth));
      layer.mask.setAttribute("height", String(viewportHeight));
      layer.maskField.setAttribute("x", "0");
      layer.maskField.setAttribute("y", "0");
      layer.maskField.setAttribute("width", String(viewportWidth));
      layer.maskField.setAttribute("height", String(viewportHeight));
      for (const cutout of layer.mask.querySelectorAll(
        "[data-inline-popover-obstruction]",
      )) {
        cutout.remove();
      }
      for (const obstruction of obstructions) {
        const left = Math.max(
          0,
          obstruction.left - TRACE_OBSTRUCTION_GAP,
        );
        const top = Math.max(0, obstruction.top - TRACE_OBSTRUCTION_GAP);
        const right = Math.min(
          viewportWidth,
          obstruction.right + TRACE_OBSTRUCTION_GAP,
        );
        const bottom = Math.min(
          viewportHeight,
          obstruction.bottom + TRACE_OBSTRUCTION_GAP,
        );
        const cutout = document.createElementNS(svgNamespace, "rect");
        cutout.setAttribute("data-inline-popover-obstruction", "");
        cutout.setAttribute("x", String(left));
        cutout.setAttribute("y", String(top));
        cutout.setAttribute("width", String(right - left));
        cutout.setAttribute("height", String(bottom - top));
        cutout.setAttribute("fill", "black");
        layer.mask.append(cutout);
      }
    }
  };

  const geometryFor = (
    entry,
    viewportWidth,
    viewportHeight,
    obstructions = obstructionRects(viewportWidth, viewportHeight),
  ) => {
    if (
      !entry.trigger.isConnected ||
      entry.trigger.closest("[popover]") ||
      entry.trigger.getClientRects().length === 0
    ) {
      return null;
    }
    const triggerRect = entry.trigger.getBoundingClientRect();
    if (
      obstructions.some((obstruction) =>
        rectanglesOverlap(triggerRect, obstruction),
      )
    ) {
      return null;
    }
    const host = hostFor(entry.trigger);
    const hostRect = host?.getBoundingClientRect() || triggerRect;
    const leftSpace = hostRect.left - EDGE_INSET - SOURCE_GAP;
    const rightSpace =
      viewportWidth - hostRect.right - EDGE_INSET - SOURCE_GAP;
    const sides = {};
    if (leftSpace >= MIN_RAIL_WIDTH) {
      sides.left = Math.min(MAX_RAIL_WIDTH, Math.floor(leftSpace));
    }
    if (rightSpace >= MIN_RAIL_WIDTH) {
      sides.right = Math.min(MAX_RAIL_WIDTH, Math.floor(rightSpace));
    }
    // A declared side is art direction, not a guess: when the authored page
    // has no conventional gutter on that edge (a full-bleed hero is the
    // canonical case), reserve an edge rail anyway. The obstacle pass still
    // prevents that deliberate placement from covering protected content.
    if (entry.preferredSide && !sides[entry.preferredSide]) {
      sides[entry.preferredSide] = Math.min(
        MAX_RAIL_WIDTH,
        Math.floor(viewportWidth / 3),
      );
    }
    const slotRect = entry.preferredSlot?.getClientRects().length
      ? entry.preferredSlot.getBoundingClientRect()
      : null;
    if (!slotRect && !sides.left && !sides.right) return null;

    const center = triggerRect.top + triggerRect.height / 2;
    const inBand =
      triggerRect.bottom >= SAFE_TOP &&
      triggerRect.top <= viewportHeight - SAFE_BOTTOM;
    return {
      entry,
      triggerRect,
      hostRect,
      sides,
      slotRect,
      center,
      inBand,
      distance: Math.abs(center - viewportHeight * 0.44),
    };
  };

  const prepareRail = (geometry, option, viewportWidth) => {
    const { entry } = geometry;
    const { side, width } = option;
    const x = Number.isFinite(option.x)
      ? option.x
      : side === "left"
        ? EDGE_INSET
        : viewportWidth - EDGE_INSET - width;
    entry.panel.setAttribute("data-inline-popover-rail-active", "");
    entry.panel.setAttribute("data-inline-popover-side", side);
    // Automatic gutter notes complement the prose without seizing focus. The
    // compact overlay restores its authored dialog role in resetRail().
    entry.panel.setAttribute("role", "note");
    entry.panel.style.setProperty("--inline-popover-rail-x", `${x}px`);
    entry.panel.style.setProperty(
      "--inline-popover-rail-y",
      `${SAFE_TOP}px`,
    );
    entry.panel.style.setProperty(
      "--inline-popover-rail-width",
      `${width}px`,
    );

    if (!entry.rail || entry.panel.getAttribute("popover") !== "manual") {
      if (entry.panel.matches(":popover-open")) entry.panel.hidePopover();
      entry.panel.setAttribute("popover", "manual");
      entry.rail = true;
    }
    if (!entry.panel.matches(":popover-open")) {
      try {
        entry.panel.showPopover({ source: entry.trigger });
      } catch {
        resetRail(entry);
        return null;
      }
    }
    const height = Math.ceil(entry.panel.getBoundingClientRect().height);
    return { ...option, side, width, height, x };
  };

  const cardRectAt = (choice, y) => ({
    left: choice.x,
    right: choice.x + choice.width,
    top: y,
    bottom: y + choice.height,
  });

  const cardClearsObstructions = (choice, y, obstructions) =>
    !obstructions.some((obstruction) =>
      rectanglesOverlap(cardRectAt(choice, y), obstruction, OBSTRUCTION_GAP),
    );

  const yOptionsFor = (
    choice,
    desiredY,
    minimumY,
    latestOrderedY,
    bottom,
    obstructions,
  ) => {
    const maximumY = bottom - choice.height;
    if (maximumY < minimumY) return [];
    const points = [SAFE_TOP, desiredY, minimumY, latestOrderedY, maximumY];
    for (const obstruction of obstructions) {
      const cardBand = {
        left: choice.x,
        right: choice.x + choice.width,
        top: SAFE_TOP,
        bottom,
      };
      if (!rectanglesOverlap(cardBand, obstruction, OBSTRUCTION_GAP)) continue;
      points.push(
        obstruction.top - OBSTRUCTION_GAP - choice.height,
        obstruction.bottom + OBSTRUCTION_GAP,
      );
    }
    return [
      ...new Set(
        points.map((point) =>
          Math.max(minimumY, Math.min(maximumY, point)),
        ),
      ),
    ]
      .filter((y) => cardClearsObstructions(choice, y, obstructions))
      .sort(
        (first, second) =>
          Math.abs(first - desiredY) - Math.abs(second - desiredY) ||
          first - second,
      );
  };

  const choiceHasOpenSlot = (choice, bottom, obstructions) => {
    const desiredY = choice.preferredY ?? SAFE_TOP;
    const options = yOptionsFor(
      choice,
      desiredY,
      SAFE_TOP,
      bottom - choice.height,
      bottom,
      obstructions,
    );
    return choice.authoredSlot
      ? options.some(
          (y) => Math.abs(y - desiredY) <= MAX_AUTHORED_SLOT_DRIFT,
        )
      : options.length > 0;
  };

  const panelPositions = (items, bottom, obstructions) => {
    if (!items.length) return true;
    const verticalTarget = (item) =>
      item.choice.preferredY ?? item.geometry.center;
    items.sort((a, b) => verticalTarget(a) - verticalTarget(b));
    const capacity = bottom - SAFE_TOP;
    const totalHeight = items.reduce(
      (sum, item) => sum + item.choice.height,
      0,
    );
    const freeSpace = Math.max(0, capacity - totalHeight);
    const preferredGap =
      items.length === 1
        ? 0
        : Math.min(
            freeSpace / (items.length - 1),
            Math.max(
              MIN_CARD_GAP,
              Math.min(160, freeSpace / (items.length + 0.5)),
            ),
          );

    // Blend source affinity with slots assigned across the whole two-rail
    // constellation. The source still determines order, while the slot term
    // supplies the breathing room a pure collision pass cannot create. A
    // page-owned placement marker supplies its own ideal instead; the same
    // solver is still free to move it away from protected content and notes.
    items.forEach((item, index) => {
      const slotCenter =
        item.slotCenter ??
        SAFE_TOP + (capacity * (index + 1)) / (items.length + 1);
      const center = item.geometry.center * 0.42 + slotCenter * 0.58;
      const desiredY = item.choice.preferredY ?? center - item.choice.height / 2;
      item.desiredY = Math.max(
        SAFE_TOP,
        Math.min(bottom - item.choice.height, desiredY),
      );
    });

    const solve = (gap) => {
      let best = null;
      let bestCost = Number.POSITIVE_INFINITY;
      const positions = [];
      const visit = (index, previousBottom, cost) => {
        if (cost >= bestCost) return;
        if (index === items.length) {
          best = [...positions];
          bestCost = cost;
          return;
        }
        const item = items[index];
        const remainingHeight = items
          .slice(index + 1)
          .reduce((sum, remaining) => sum + remaining.choice.height, 0);
        const remainingGaps = gap * (items.length - index - 1);
        const latestOrderedY =
          bottom - item.choice.height - remainingHeight - remainingGaps;
        const minimumY =
          index === 0 ? SAFE_TOP : previousBottom + gap;
        for (const y of yOptionsFor(
          item.choice,
          item.desiredY,
          minimumY,
          latestOrderedY,
          bottom,
          obstructions,
        )) {
          positions.push(y);
          visit(
            index + 1,
            y + item.choice.height,
            cost + Math.abs(y - item.desiredY),
          );
          positions.pop();
        }
      }
      visit(0, SAFE_TOP, 0);
      return best;
    };

    const gaps = [
      preferredGap,
      Math.min(preferredGap, MIN_CARD_GAP),
      0,
    ].filter((gap, index, all) => all.indexOf(gap) === index);
    const positions = gaps.map(solve).find(Boolean);
    if (!positions) return false;
    if (
      positions.some(
        (y, index) =>
          items[index].choice.authoredSlot &&
          Math.abs(y - items[index].choice.preferredY) >
            MAX_AUTHORED_SLOT_DRIFT,
      )
    ) {
      return false;
    }
    items.forEach((item, index) => {
      item.y = positions[index];
      delete item.desiredY;
    });
    return true;
  };

  const sourceFragmentFor = (entry, targetY) => {
    const fragments = [...entry.trigger.getClientRects()];
    return fragments.reduce((closest, fragment) => {
      if (!closest) return fragment;
      const distance = Math.abs(fragment.top + fragment.height / 2 - targetY);
      const closestDistance = Math.abs(
        closest.top + closest.height / 2 - targetY,
      );
      return distance < closestDistance ? fragment : closest;
    }, null);
  };

  const drawTrace = (item, lane) => {
    const { entry } = item.geometry;
    const { side, width, height, x } = item.choice;
    const sideEndY = item.y + Math.min(34, height / 2);
    const fragment = sourceFragmentFor(entry, sideEndY);
    if (
      !fragment ||
      fragment.bottom < 0 ||
      fragment.top > document.documentElement.clientHeight
    ) {
      clearTrace(entry);
      return;
    }
    const startX =
      side === "left"
        ? fragment.left - LEFT_SOURCE_PORT_GAP
        : fragment.right + RIGHT_SOURCE_PORT_GAP;
    const startY = fragment.top + fragment.height / 2;
    entry.port.setAttribute("cx", startX.toFixed(1));
    entry.port.setAttribute("cy", startY.toFixed(1));

    // An authored edge rail can overlap the source's horizontal band even
    // though the card and prose are vertically separate. In that art-directed
    // case, meet the card's top/bottom edge; forcing a side-edge connector
    // would send a trace back through the very label it annotates.
    if (startX >= x && startX <= x + width) {
      const cardCenter = item.y + height / 2;
      const endY = startY < cardCenter ? item.y : item.y + height;
      const approachY = endY + (startY < cardCenter ? -10 : 10);
      const endX = Math.max(x + 16, Math.min(x + width - 16, startX));
      entry.path.setAttribute(
        "d",
        `M ${startX.toFixed(1)} ${startY.toFixed(1)} V ${approachY.toFixed(1)} H ${endX.toFixed(1)} V ${endY.toFixed(1)}`,
      );
      return;
    }

    const endY = sideEndY;
    const endX = side === "left" ? x + width : x;
    const busX =
      side === "left"
        ? Math.max(endX + 10, startX - 16 - lane * 8)
        : Math.min(endX - 10, startX + 16 + lane * 8);
    entry.path.setAttribute(
      "d",
      `M ${startX.toFixed(1)} ${startY.toFixed(1)} H ${busX.toFixed(1)} V ${endY.toFixed(1)} H ${endX.toFixed(1)}`,
    );
  };

  const scoreSelection = (
    selection,
    priorSelected,
    interaction,
    viewportHeight,
  ) => {
    const readingLine = viewportHeight * 0.44;
    let score = 0;

    for (const item of selection) {
      const { geometry, choice } = item;
      const proximity = Math.max(
        0,
        1 - geometry.distance / (viewportHeight * 0.72),
      );
      score += 108 + proximity * 72;
      if (priorSelected.has(geometry.entry)) score += 32;
      if (geometry.entry.previousSide === choice.side) score += 18;
      if (
        choice.side ===
        (entries.indexOf(geometry.entry) % 2 ? "left" : "right")
      ) {
        score += 5;
      }
      score += (choice.width - MIN_RAIL_WIDTH) * 0.08;

      // When a reader summons a note in the middle of a sequence, gently
      // favor keeping the already-visible note ahead of it. This makes the
      // rails feel like they are following the direction of reading instead
      // of snapping back to material the reader has passed.
      if (interaction && geometry.entry !== interaction.entry) {
        if (geometry.center > interaction.center) score += 14;
        if (
          priorSelected.has(geometry.entry) &&
          geometry.center > interaction.center
        ) {
          score += 20;
        }
      }
    }

    if (selection.length > 1) {
      const ordered = [...selection].sort(
        (a, b) => a.geometry.center - b.geometry.center,
      );
      const span =
        ordered.at(-1).geometry.center - ordered[0].geometry.center;
      const nearestGap = ordered
        .slice(1)
        .reduce(
          (nearest, item, index) =>
            Math.min(
              nearest,
              item.geometry.center - ordered[index].geometry.center,
            ),
          viewportHeight,
        );
      score += (span / viewportHeight) * 205;
      score += (nearestGap / viewportHeight) * 78;
      if (
        !interaction &&
        ordered[0].geometry.center < readingLine &&
        ordered.at(-1).geometry.center > readingLine
      ) {
        score += 26;
      }

      const sides = new Set(selection.map((item) => item.choice.side));
      if (sides.size > 1) score += 44;
    }

    return score;
  };

  // At most 3^8 assignments (hidden/left/right) are considered. That tiny,
  // bounded search lets selection and collision constraints be decided
  // together, rather than allowing document order to greedily fill the rails.
  const chooseSelection = (
    candidates,
    capacity,
    visibleLimit,
    priorSelected,
    interaction,
    viewportHeight,
  ) => {
    const selected = [];
    const used = { left: 0, right: 0 };
    const counts = { left: 0, right: 0 };
    let best = [];
    let bestScore = Number.NEGATIVE_INFINITY;

    const visit = (index) => {
      if (index === candidates.length) {
        if (!selected.length) return;
        if (
          interaction &&
          !selected.some((item) => item.geometry === interaction)
        ) {
          return;
        }
        const score = scoreSelection(
          selected,
          priorSelected,
          interaction,
          viewportHeight,
        );
        if (score > bestScore) {
          bestScore = score;
          best = selected.map((item) => ({ ...item, y: SAFE_TOP }));
        }
        return;
      }

      const geometry = candidates[index];
      const mandatory = geometry === interaction;
      if (!mandatory) visit(index + 1);
      if (selected.length >= visibleLimit) return;

      for (const choice of geometry.choices) {
        const { side } = choice;
        const extra = (counts[side] ? MIN_CARD_GAP : 0) + choice.height;
        if (used[side] + extra > capacity) continue;
        used[side] += extra;
        counts[side] += 1;
        selected.push({ geometry, choice });
        visit(index + 1);
        selected.pop();
        counts[side] -= 1;
        used[side] -= extra;
      }
    };

    visit(0);
    return best;
  };

  const layoutRail = () => {
    animationFrame = 0;
    if (railsRevealed()) readerConsented = true;
    if (!desktop.matches || !railsAreActivated()) {
      deactivate();
      return;
    }

    // Opt the rail surface into its final styling before showPopover(). That
    // lets each newly selected card enter through its @starting-style
    // clip/fade instead of appearing fully drawn for its first frame.
    document.documentElement.setAttribute("data-inline-popover-rails", "");

    const viewportWidth = document.documentElement.clientWidth;
    const viewportHeight = document.documentElement.clientHeight;
    const bottom = viewportHeight - SAFE_BOTTOM;
    const capacity = bottom - SAFE_TOP;
    const visibleLimit = Math.max(
      1,
      Math.min(MAX_VISIBLE, Math.round(capacity / TARGET_VERTICAL_SLOT)),
    );
    const priorSelected = new Set(
      entries.filter((entry) => entry.rail),
    );
    const interaction = interactionEntry();
    const obstructions = obstructionRects(viewportWidth, viewportHeight);
    updateLineMasks(obstructions, viewportWidth, viewportHeight);

    const geometries = [];
    for (const entry of entries) {
      const geometry = geometryFor(
        entry,
        viewportWidth,
        viewportHeight,
        obstructions,
      );
      entry.geometry = geometry;
      if (!geometry || !geometry.inBand) dismissed.delete(entry);
      if (
        geometry &&
        !dismissed.has(entry) &&
        (geometry.inBand || entry === interaction)
      ) {
        geometries.push(geometry);
      }
    }

    geometries.sort((a, b) => {
      const aPriority =
        a.entry === interaction ? 0 : priorSelected.has(a.entry) ? 1 : 2;
      const bPriority =
        b.entry === interaction ? 0 : priorSelected.has(b.entry) ? 1 : 2;
      return aPriority - bPriority || a.distance - b.distance;
    });

    const measured = geometries.slice(0, MAX_CANDIDATES);
    if (!interaction && geometries.length > measured.length) {
      const orderedBySource = [...geometries].sort(
        (a, b) => a.center - b.center,
      );
      const bandEdges = [orderedBySource[0], orderedBySource.at(-1)];

      // A dense, short page can leave every trigger onscreen at once. Keep
      // the solver bounded, but reserve room for both ends of that visible
      // sequence so its own spread scoring can choose a true top/bottom
      // constellation instead of only seeing the middle cluster.
      for (const edge of bandEdges) {
        if (measured.includes(edge)) continue;
        let replace = measured.length - 1;
        while (
          replace >= 0 &&
          (priorSelected.has(measured[replace].entry) ||
            bandEdges.includes(measured[replace]))
        ) {
          replace -= 1;
        }
        if (replace >= 0) measured[replace] = edge;
      }
    }
    for (const geometry of measured) {
      geometry.choices = [];
      const addChoice = (option) => {
        const choice = prepareRail(geometry, option, viewportWidth);
        if (
          choice &&
          choice.height <= capacity &&
          choiceHasOpenSlot(choice, bottom, obstructions)
        ) {
          geometry.choices.push(choice);
        }
      };

      if (geometry.slotRect) {
        const width = Math.max(
          MIN_SLOT_WIDTH,
          Math.min(MAX_RAIL_WIDTH, Math.round(geometry.slotRect.width)),
        );
        const x = Math.max(
          EDGE_INSET,
          Math.min(
            viewportWidth - EDGE_INSET - width,
            Math.round(geometry.slotRect.left),
          ),
        );
        const triggerCenterX =
          geometry.triggerRect.left + geometry.triggerRect.width / 2;
        addChoice({
          side: x + width / 2 < triggerCenterX ? "left" : "right",
          width,
          x,
          preferredY: geometry.slotRect.top,
          authoredSlot: true,
        });
      }

      // A usable authored slot (or side) is the only option the solver should
      // retain. Avoid opening the card elsewhere merely to measure discarded
      // choices; this also anchors its entrance to the edge it grows from.
      if (!geometry.choices.length) {
        const preferredSide = geometry.entry.preferredSide;
        const preferredWidth = preferredSide
          ? geometry.sides[preferredSide]
          : null;
        if (preferredWidth) addChoice({ side: preferredSide, width: preferredWidth });
        if (!preferredSide || !geometry.choices.length) {
          for (const [side, width] of Object.entries(geometry.sides)) {
            if (side !== preferredSide) addChoice({ side, width });
          }
        }
      }
    }

    const candidates = measured.filter((geometry) => geometry.choices.length);
    const interactionGeometry =
      candidates.find((geometry) => geometry.entry === interaction) || null;
    let selected = chooseSelection(
      candidates,
      capacity,
      visibleLimit,
      priorSelected,
      interactionGeometry,
      viewportHeight,
    );

    for (const item of selected) {
      // The last measured option may not be the winning one; restore the
      // winner before the final placement pass.
      const choice = prepareRail(
        item.geometry,
        item.choice,
        viewportWidth,
      );
      if (choice) item.choice = choice;
      item.geometry.entry.previousSide = item.choice.side;
    }

    [...selected]
      .sort((a, b) => a.geometry.center - b.geometry.center)
      .forEach((item, index) => {
        item.slotCenter =
          SAFE_TOP + (capacity * (index + 0.5)) / selected.length;
      });
    const left = selected.filter((item) => item.choice.side === "left");
    const right = selected.filter((item) => item.choice.side === "right");
    const fitSide = (sideItems) => {
      while (
        sideItems.length &&
        !panelPositions(sideItems, bottom, obstructions)
      ) {
        const nonInteraction = sideItems.filter(
          (item) => item.geometry.entry !== interaction,
        );
        const removable = nonInteraction.length ? nonInteraction : sideItems;
        const furthest = removable.reduce((candidate, item) =>
          !candidate || item.geometry.distance > candidate.geometry.distance
            ? item
            : candidate,
        );
        sideItems.splice(sideItems.indexOf(furthest), 1);
      }
    };
    for (const sideItems of [left, right]) fitSide(sideItems);

    // Free-placement slots can sit on opposite connector sides while their
    // rectangles still share the same interior patch of a composition. Catch
    // that genuine 2-D collision after the per-side solve. Interaction wins;
    // otherwise preserve an already-settled card, then the nearer source.
    const collision = () => {
      const items = [...left, ...right];
      for (let first = 0; first < items.length; first += 1) {
        for (let second = first + 1; second < items.length; second += 1) {
          if (
            rectanglesOverlap(
              cardRectAt(items[first].choice, items[first].y),
              cardRectAt(items[second].choice, items[second].y),
              MIN_CARD_GAP,
            )
          ) {
            return [items[first], items[second]];
          }
        }
      }
      return null;
    };
    for (let pair = collision(); pair; pair = collision()) {
      const nonInteraction = pair.filter(
        (item) => item.geometry.entry !== interaction,
      );
      const removable = nonInteraction.length ? nonInteraction : pair;
      const remove = removable.reduce((candidate, item) => {
        if (!candidate) return item;
        const candidateWasPresent = priorSelected.has(candidate.geometry.entry);
        const itemWasPresent = priorSelected.has(item.geometry.entry);
        if (candidateWasPresent !== itemWasPresent) {
          return candidateWasPresent ? item : candidate;
        }
        return item.geometry.distance > candidate.geometry.distance
          ? item
          : candidate;
      }, null);
      const sideItems = left.includes(remove) ? left : right;
      sideItems.splice(sideItems.indexOf(remove), 1);
      fitSide(sideItems);
    }
    selected = [...left, ...right];
    const selectedEntries = new Set(
      selected.map((item) => item.geometry.entry),
    );
    for (const entry of entries) {
      if (!selectedEntries.has(entry)) resetRail(entry);
    }

    if (!selected.length) {
      document.documentElement.removeAttribute("data-inline-popover-rails");
      focused = null;
      return;
    }
    for (const items of [left, right]) {
      items.forEach((item, lane) => {
        item.geometry.entry.panel.style.setProperty(
          "--inline-popover-rail-y",
          `${Math.round(item.y)}px`,
        );
        drawTrace(item, lane);
      });
    }

    focused = selectedEntries.has(hovered) ? hovered : null;

    for (const item of selected) {
      const { entry } = item.geometry;
      const current = entry === focused;
      entry.panel.toggleAttribute("data-inline-popover-focused", current);
      entry.path.toggleAttribute("data-inline-popover-focused", current);
      entry.port.toggleAttribute("data-inline-popover-focused", current);
      entry.trigger.toggleAttribute("data-inline-popover-current", current);
      const targetLayer = current
        ? focusedLineLayer.traces
        : ambientLineLayer.traces;
      if (entry.path.parentNode !== targetLayer) {
        targetLayer.append(entry.path, entry.port);
      }
    }
  };

  const schedule = () => {
    if (!animationFrame) animationFrame = requestAnimationFrame(layoutRail);
  };

  const handles = (trigger, panel) => {
    const entry = entryForPanel.get(panel);
    if (
      !desktop.matches ||
      !railsAreActivated() ||
      !entry ||
      entry.trigger !== trigger
    ) {
      return false;
    }
    const geometry = geometryFor(
      entry,
      document.documentElement.clientWidth,
      document.documentElement.clientHeight,
    );
    return Boolean(geometry?.inBand);
  };

  const toggle = (panel) => {
    const entry = entryForPanel.get(panel);
    if (!entry) return;
    if (entry.rail && panel.matches(":popover-open")) {
      // Pointer arrival may have opened a previously hidden annotation before
      // the ensuing click lands. Treat that click as a pin, not an accidental
      // request to immediately close the note it just revealed.
      if (hovered === entry) {
        forced = entry;
        schedule();
        return;
      }
      dismissed.add(entry);
      if (hovered === entry) hovered = null;
      if (panelFocused === entry) panelFocused = null;
      if (forced === entry) forced = null;
      resetRail(entry);
    } else {
      dismissed.delete(entry);
      forced = entry;
    }
    schedule();
  };

  const dismiss = (panel, returnFocus = false) => {
    const entry = entryForPanel.get(panel);
    if (!entry?.rail) return false;
    dismissed.add(entry);
    if (hovered === entry) hovered = null;
    if (panelFocused === entry) panelFocused = null;
    if (forced === entry) forced = null;
    requestAnimationFrame(() => {
      schedule();
      if (returnFocus && entry.trigger.isConnected) {
        entry.trigger.focus({ preventScroll: true });
      }
    });
    return true;
  };

  const dismissFocused = () => {
    const entry =
      (focused?.rail && focused) || entries.find((candidate) => candidate.rail);
    if (!entry) return false;
    dismissed.add(entry);
    if (hovered === entry) hovered = null;
    if (panelFocused === entry) panelFocused = null;
    if (forced === entry) forced = null;
    resetRail(entry);
    if (entry.trigger.isConnected) entry.trigger.focus({ preventScroll: true });
    schedule();
    return true;
  };

  const holdHover = (entry) => {
    window.clearTimeout(hoverReleaseTimer);
    hoverReleaseTimer = 0;
    readerConsented = true;
    dismissed.delete(entry);
    hovered = entry;
    schedule();
  };

  const releaseHover = (entry, event) => {
    if (
      event.relatedTarget instanceof Node &&
      (entry.panel.contains(event.relatedTarget) ||
        entry.trigger.contains(event.relatedTarget))
    ) {
      return;
    }
    window.clearTimeout(hoverReleaseTimer);
    hoverReleaseTimer = window.setTimeout(() => {
      hoverReleaseTimer = 0;
      if (entry.panel.matches(":hover") || entry.trigger.matches(":hover")) {
        return;
      }
      if (hovered === entry) hovered = null;
      schedule();
    }, HOVER_GRACE_MS);
  };

  for (const entry of entries) {
    entry.panel.addEventListener("pointerenter", () => holdHover(entry));
    entry.panel.addEventListener("pointerleave", (event) =>
      releaseHover(entry, event),
    );
    entry.panel.addEventListener("focusin", () => {
      panelFocused = entry;
      schedule();
    });
    entry.panel.addEventListener("focusout", (event) => {
      if (
        event.relatedTarget instanceof Node &&
        (entry.panel.contains(event.relatedTarget) ||
          entry.trigger.contains(event.relatedTarget))
      ) {
        return;
      }
      if (panelFocused === entry) panelFocused = null;
      schedule();
    });
    entry.trigger.addEventListener("pointerenter", () => holdHover(entry));
    entry.trigger.addEventListener("pointerleave", (event) =>
      releaseHover(entry, event),
    );
    entry.trigger.addEventListener("focus", () => {
      // Close/Escape returns keyboard focus to the source. Keep that useful
      // focus move from being mistaken for a request to reopen immediately;
      // an intentional click still clears dismissal in toggle().
      if (dismissed.has(entry)) return;
      readerConsented = true;
      forced = entry;
      schedule();
    });
    entry.trigger.addEventListener("blur", (event) => {
      if (
        event.relatedTarget instanceof Node &&
        entry.panel.contains(event.relatedTarget)
      ) {
        return;
      }
      if (forced === entry) forced = null;
      schedule();
    });
  }

  window.addEventListener(
    "scroll",
    () => {
      if (railsRevealed()) readerConsented = true;
      if (
        !hovered &&
        !panelFocused &&
        forced &&
        document.activeElement !== forced.trigger
      ) {
        forced = null;
      }
      schedule();
    },
    { passive: true },
  );
  window.addEventListener("resize", schedule, { passive: true });
  document.addEventListener("input", schedule);
  document.addEventListener("change", schedule);
  desktop.addEventListener("change", schedule);
  if ("ResizeObserver" in window) {
    const resizeObserver = new ResizeObserver(schedule);
    for (const entry of entries) resizeObserver.observe(entry.panel);
    for (const obstruction of document.querySelectorAll(
      "[data-dont-obstruct]",
    )) {
      resizeObserver.observe(obstruction);
    }
    for (const slot of placementSlots.values()) resizeObserver.observe(slot);
  }
  document.fonts?.ready.then(schedule);
  schedule();

  return { handles, toggle, dismiss, dismissFocused };
};

const inlinePopoverRails = createInlinePopoverRails();

const canModal = (dialog) =>
  dialog instanceof HTMLDialogElement && typeof dialog.showModal === "function";

// Native showModal focuses the first autofocus/focusable child; make it
// deterministic and prefer an explicit autofocus, then the close control.
const focusInside = (dialog) => {
  const target =
    dialog.querySelector("[autofocus]") ||
    dialog.querySelector("[data-modal-close]") ||
    dialog;
  try {
    target.focus({ preventScroll: true });
  } catch {
    /* a detached or hidden target is not worth failing the open over */
  }
};

const open = (dialog, trigger) => {
  if (!canModal(dialog) || dialog.open) return false;
  if (trigger) openers.set(dialog, trigger);
  dialog.showModal();
  focusInside(dialog);
  dialog.dispatchEvent(new CustomEvent("modal:open", { bubbles: true }));
  return true;
};

document.addEventListener("click", (event) => {
  if (!(event.target instanceof Element)) return;

  const popoverCloser = event.target.closest("[data-inline-popover-close]");
  if (popoverCloser) {
    const panel = popoverCloser.closest("[data-inline-popover-panel]");
    inlinePopoverRails.dismiss(panel, event.detail === 0);
  }

  const popoverTrigger = event.target.closest("[data-inline-popover-trigger]");
  if (popoverTrigger) {
    const popover = popoverFor(popoverTrigger);
    if (!canPopover(popover)) return;
    event.preventDefault();
    if (inlinePopoverRails.handles(popoverTrigger, popover)) {
      inlinePopoverRails.toggle(popover);
      return;
    }
    if (popover.matches(":popover-open")) {
      popover.hidePopover();
    } else {
      // Open after the activating click finishes. Otherwise that same click
      // is also interpreted as an outside click against the newly open panel.
      setTimeout(() => {
        if (popover.isConnected && !popover.matches(":popover-open")) {
          popover.showPopover({ source: popoverTrigger });
        }
      }, 0);
    }
    return;
  }

  const trigger = event.target.closest("[data-modal-open]");
  if (trigger) {
    // Let the browser keep new-tab / modified clicks as real navigations.
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }
    const dialog = document.getElementById(trigger.getAttribute("data-modal-open"));
    // No dialog, or a browser without showModal: don't swallow the click —
    // the trigger's own href navigates instead (progressive enhancement).
    if (!canModal(dialog)) return;
    event.preventDefault();
    open(dialog, trigger);
    return;
  }

  const closer = event.target.closest("[data-modal-close]");
  if (closer) {
    closer.closest("dialog")?.close();
    return;
  }

  // A click whose target is the dialog itself landed on the backdrop around
  // the panel (the panel and its children are separate targets).
  if (event.target instanceof HTMLDialogElement && event.target.hasAttribute("data-modal")) {
    event.target.close();
  }
});

document.addEventListener("keydown", (event) => {
  if (
    event.key === "Escape" &&
    !document.querySelector("dialog[open]") &&
    inlinePopoverRails.dismissFocused()
  ) {
    event.preventDefault();
    return;
  }
  if (event.key !== " " || !(event.target instanceof Element)) return;
  const trigger = event.target.closest("[data-inline-popover-trigger]");
  if (!trigger) return;
  event.preventDefault();
  trigger.click();
});

for (const popover of document.querySelectorAll("[popover]")) {
  if (!popover.id) continue;
  popover.addEventListener("toggle", () => setPopoverExpanded(popover));
  setPopoverExpanded(popover);
}

for (const dialog of document.querySelectorAll("dialog[data-modal]")) {
  // One close handler covers every route out — button, backdrop, Escape, or a
  // companion calling close() — so focus return has a single home.
  dialog.addEventListener("close", () => {
    dialog.dispatchEvent(new CustomEvent("modal:close", { bubbles: true }));
    const opener = openers.get(dialog);
    openers.delete(dialog);
    if (opener && opener.isConnected) opener.focus();
  });
  if (dialog.hasAttribute("data-modal-open-on-load")) open(dialog, null);
}

for (const trigger of document.querySelectorAll("[data-modal-open]")) {
  const dialog = document.getElementById(trigger.getAttribute("data-modal-open"));
  if (dialog) {
    trigger.setAttribute("aria-haspopup", "dialog");
    trigger.setAttribute("aria-controls", dialog.id);
  }
}
