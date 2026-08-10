// Vimium-style link hints for the whole site, independent of appearance.

const HINT_CHARS = "sadfjklewcmpgh";

export function createHints() {
  let state = null;

  const labels = (count) => {
    let length = 1;
    while (Math.pow(HINT_CHARS.length, length) < count) length += 1;
    const result = [];
    const grow = (prefix) => {
      if (result.length >= count) return;
      if (prefix.length === length) {
        result.push(prefix);
        return;
      }
      for (const char of HINT_CHARS) grow(prefix + char);
    };
    grow("");
    return result;
  };

  const targets = () =>
    [
      ...document.querySelectorAll(
        "a[href], button, summary, input, select, textarea, [role='button'], audio[controls]"
      ),
    ].filter((element) => {
      if (element.disabled || element.closest("[hidden]")) return false;
      const box = element.getBoundingClientRect();
      return (
        box.width > 1 &&
        box.height > 1 &&
        box.bottom > 0 &&
        box.right > 0 &&
        box.top < innerHeight &&
        box.left < innerWidth &&
        getComputedStyle(element).visibility !== "hidden"
      );
    });

  const end = () => {
    if (!state) return;
    for (const overlay of state.overlays) overlay.remove();
    state = null;
    removeEventListener("resize", end);
  };

  const openPopoverOf = (element) => {
    const popover = element.closest("[popover]");
    try {
      return popover && popover.matches(":popover-open") ? popover : null;
    } catch {
      return null;
    }
  };

  const paint = () => {
    for (const { chip, label } of state.chips) {
      const live = label.startsWith(state.typed);
      chip.style.display = live ? "" : "none";
      for (let i = 0; i < chip.children.length; i++) {
        chip.children[i].classList.toggle(
          "key-hint-typed",
          live && i < state.typed.length
        );
      }
    }
  };

  const start = () => {
    end();
    const interactables = targets();
    if (!interactables.length) return;
    const hintLabels = labels(interactables.length);
    const overlays = new Map();
    const overlayIn = (host) => {
      let entry = overlays.get(host);
      if (!entry) {
        const overlay = document.createElement("div");
        overlay.className = "key-hints";
        overlay.setAttribute("aria-hidden", "true");
        host.appendChild(overlay);
        entry = { overlay, origin: overlay.getBoundingClientRect() };
        overlays.set(host, entry);
      }
      return entry;
    };
    const chips = interactables.map((target, index) => {
      const { overlay, origin } = overlayIn(
        openPopoverOf(target) || document.body
      );
      const box = target.getBoundingClientRect();
      const chip = document.createElement("span");
      chip.className = "key-hint";
      for (const char of hintLabels[index]) {
        const key = document.createElement("span");
        key.textContent = char;
        chip.appendChild(key);
      }
      chip.style.top = `${Math.max(2, box.top - origin.top - 8)}px`;
      chip.style.left = `${Math.max(2, box.left - origin.left - 6)}px`;
      overlay.appendChild(chip);
      return { chip, target, label: hintLabels[index] };
    });
    state = {
      overlays: [...overlays.values()].map(({ overlay }) => overlay),
      chips,
      typed: "",
    };
    addEventListener("resize", end);
  };

  const fire = (target) => {
    end();
    if (target.matches("input, textarea, select")) target.focus();
    else target.click();
  };

  const key = (event) => {
    if (!state) return;
    if (event.ctrlKey || event.altKey || event.metaKey) {
      end();
      return;
    }
    if (event.key === "Shift") return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      end();
      return;
    }
    if (event.key === "Backspace") {
      state.typed = state.typed.slice(0, -1);
      paint();
      return;
    }
    const pressed = event.key.toLowerCase();
    if (pressed.length !== 1 || !HINT_CHARS.includes(pressed)) {
      end();
      return;
    }
    const next = state.typed + pressed;
    const hit = state.chips.find(({ label }) => label === next);
    if (hit) {
      fire(hit.target);
      return;
    }
    if (!state.chips.some(({ label }) => label.startsWith(next))) return;
    state.typed = next;
    paint();
  };

  return { end, isActive: () => !!state, key, start };
}
