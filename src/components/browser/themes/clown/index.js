// Clown's theme-package adapter. Physics listeners remain installed after
// first use, but do nothing while this adapter is inactive.

export const id = "clown";
export const colorScheme = "light";
export const music = Object.freeze({ kind: "audio", asset: "music" });

export function tone({ tone }) {
  tone({ f: 400, d: 0.32, w: "sine", v: 0.5, glide: 950 });
  tone({ f: 233, t: 0.4, d: 0.16, w: "square", v: 0.5, glide: 175 });
  tone({ f: 233, t: 0.62, d: 0.18, w: "square", v: 0.5, glide: 170 });
}

const GRAVITY = 1800;
const BALL_COUNT = 12;
const props = new Set();
const suppressClicks = new WeakSet();
let active = false;
let propFor = new WeakMap();
let juggleRaf = 0;
let juggleLastFrame = 0;
let juggleOverlay = null;

const celebrate = (reducedMotion) => {
  if (reducedMotion) return;
  dropBalls();
  if (!("animate" in Element.prototype)) return;
  const colors = ["#c81e2b", "#2f6bb0", "#157a3d", "#b0801c"];
  for (let i = 0; i < 36; i++) {
    const bit = document.createElement("span");
    const size = 6 + Math.random() * 6;
    bit.style.cssText =
      "position:fixed;top:-3vh;z-index:99;pointer-events:none;" +
      `left:${Math.random() * 100}vw;width:${size}px;height:${size * 0.6}px;` +
      `background:${colors[i % colors.length]};` +
      `border-radius:${i % 3 ? "1px" : "50%"};`;
    document.body.appendChild(bit);
    bit
      .animate(
        [
          { transform: "translateY(0) rotate(0turn)", opacity: 1 },
          {
            transform: `translateY(105vh) rotate(${1 + Math.random() * 2}turn)`,
            opacity: 0.7,
          },
        ],
        {
          duration: 1800 + Math.random() * 1600,
          delay: Math.random() * 250,
          easing: "cubic-bezier(.2,.6,.4,1)",
        }
      )
      .addEventListener("finish", () => bit.remove());
  }
};

const makeBall = (x, y, size) => {
  const element = document.createElement("span");
  element.dataset.clownBall = "";
  element.dataset.clownPhysics = "";
  element.setAttribute("aria-hidden", "true");
  overlay().appendChild(element);
  const prop = {
    source: element,
    element,
    originalStyle: null,
    round: true,
    x,
    y,
    width: size,
    height: size,
    vx: 0,
    vy: 0,
    angle: 0,
    spin: 0,
    held: false,
    pointerId: null,
    grabX: 0,
    grabY: 0,
    lastX: 0,
    lastY: 0,
    lastAt: 0,
    dragged: false,
    sleeping: false,
  };
  Object.assign(element.style, {
    position: "absolute",
    left: "0",
    top: "0",
    width: `${size}px`,
    height: `${size}px`,
    boxSizing: "border-box",
    margin: "0",
    zIndex: "1",
    pointerEvents: "auto",
    touchAction: "none",
    userSelect: "none",
    cursor: "grab",
    willChange: "transform",
  });
  props.add(prop);
  propFor.set(element, prop);
  paintProp(prop);
  return prop;
};

const dropBalls = () => {
  for (let i = 0; i < BALL_COUNT; i++) {
    const size = 28 + Math.random() * 12;
    const prop = makeBall(
      Math.random() * Math.max(0, innerWidth - size),
      -size - Math.random() * 140,
      size
    );
    prop.vx = (Math.random() - 0.5) * 520;
    prop.vy = 80 + Math.random() * 260;
    prop.spin = (Math.random() - 0.5) * 900;
  }
  ensureJuggleFrame();
};

const juggleTarget = (target) => {
  if (!(target instanceof Element)) return null;
  if (target.closest(".corner-rack, [data-no-clown-physics]")) return null;
  const link = target.closest("a");
  if (link) return link;
  const button = target.closest("button");
  if (button) return button;
  return target.closest("img, video, [data-clown-physics]");
};

const clampSpeed = (speed) => Math.max(-1800, Math.min(1800, speed));

const ensureJuggleFrame = () => {
  if (juggleRaf) return;
  juggleLastFrame = performance.now();
  juggleRaf = requestAnimationFrame(juggleFrame);
};

const copyComputedTree = (source, copy) => {
  const computed = getComputedStyle(source);
  for (let i = 0; i < computed.length; i++) {
    const name = computed[i];
    copy.style.setProperty(name, computed.getPropertyValue(name));
  }
  copy.removeAttribute("id");
  for (let i = 0; i < source.children.length; i++) {
    copyComputedTree(source.children[i], copy.children[i]);
  }
};

const overlay = () => {
  if (juggleOverlay) return juggleOverlay;
  juggleOverlay = document.createElement("div");
  juggleOverlay.dataset.clownJuggleOverlay = "";
  juggleOverlay.setAttribute("aria-hidden", "true");
  juggleOverlay.style.cssText =
    "position:fixed;inset:0;z-index:100;pointer-events:none;overflow:visible;";
  document.body.appendChild(juggleOverlay);
  return juggleOverlay;
};

const makeProp = (source) => {
  const existing = propFor.get(source);
  if (existing) return existing;
  const box = source.getBoundingClientRect();
  const element = source.cloneNode(true);
  copyComputedTree(source, element);
  element.dataset.clownProp = "";
  element.setAttribute("aria-hidden", "true");
  overlay().appendChild(element);
  const prop = {
    source,
    element,
    originalStyle: source.getAttribute("style"),
    round: false,
    x: box.left,
    y: box.top,
    width: box.width,
    height: box.height,
    vx: 0,
    vy: 0,
    angle: 0,
    spin: 0,
    held: false,
    pointerId: null,
    grabX: 0,
    grabY: 0,
    lastX: 0,
    lastY: 0,
    lastAt: 0,
    dragged: false,
    sleeping: false,
  };
  Object.assign(element.style, {
    position: "absolute",
    left: "0",
    top: "0",
    width: `${box.width}px`,
    height: `${box.height}px`,
    boxSizing: "border-box",
    margin: "0",
    zIndex: "1",
    pointerEvents: "auto",
    touchAction: "none",
    userSelect: "none",
    cursor: "grab",
    willChange: "transform",
    animation: "none",
    transition: "none",
  });
  source.style.visibility = "hidden";
  props.add(prop);
  propFor.set(source, prop);
  propFor.set(element, prop);
  paintProp(prop);
  return prop;
};

const paintProp = (prop) => {
  prop.element.style.transform =
    `translate3d(${prop.x}px, ${prop.y}px, 0) rotate(${prop.angle}deg)`;
};

const juggleFrame = (now) => {
  juggleRaf = 0;
  const dt = Math.min(0.034, (now - juggleLastFrame) / 1000 || 0.016);
  juggleLastFrame = now;
  let awake = false;
  for (const prop of props) {
    if (prop.held || prop.sleeping) {
      if (prop.held) awake = true;
      continue;
    }
    awake = true;
    prop.vy += GRAVITY * dt;
    prop.x += prop.vx * dt;
    prop.y += prop.vy * dt;
    prop.angle += prop.spin * dt;

    // The physics box is the unrotated element, but the element is PAINTED
    // rotated about its center — a spun rectangle's corners reach below
    // y + height (nearly half the width for the page-wide links a phone
    // deals in). Clamp by the rotated extent so no corner pierces the
    // floor; a circle's silhouette ignores rotation, so balls skip it.
    const rad = (prop.angle * Math.PI) / 180;
    const overhang = prop.round
      ? 0
      : Math.max(
          0,
          (Math.abs(prop.width * Math.sin(rad)) +
            Math.abs(prop.height * Math.cos(rad)) -
            prop.height) /
            2
        );
    const right = Math.max(0, innerWidth - prop.width);
    const floor = Math.max(0, innerHeight - prop.height - overhang);
    if (prop.x < 0) {
      prop.x = 0;
      prop.vx = Math.abs(prop.vx) * 0.55;
      prop.spin *= -0.7;
    } else if (prop.x > right) {
      prop.x = right;
      prop.vx = -Math.abs(prop.vx) * 0.55;
      prop.spin *= -0.7;
    }
    if (prop.y > floor) {
      prop.y = floor;
      prop.vy = -Math.abs(prop.vy) * 0.38;
      prop.vx *= 0.82;
      prop.spin *= 0.72;
      // A rectangle doesn't balance on a corner: while it sits on the
      // floor, ease it toward the nearest flat half-turn so it comes to
      // rest flush with the bottom edge.
      let flatEnough = true;
      if (!prop.round) {
        const flat = Math.round(prop.angle / 180) * 180;
        prop.angle += (flat - prop.angle) * 0.25;
        flatEnough = Math.abs(prop.angle - flat) < 0.8;
        if (flatEnough) prop.angle = flat;
      }
      if (Math.abs(prop.vy) < 70 && Math.abs(prop.vx) < 28 && flatEnough) {
        prop.vx = 0;
        prop.vy = 0;
        prop.spin = 0;
        prop.sleeping = true;
      }
    }
    paintProp(prop);
  }
  if (awake) juggleRaf = requestAnimationFrame(juggleFrame);
};

// A sleeping prop trusts the floor it fell asleep on, but viewports move
// under the pile — the phone URL bar retracts and re-expands, a rotation
// swaps the axes, a window resizes — and a stale floor leaves props
// stranded below the visible bottom. Any size change wakes everything; the
// next physics pass clamps the pile back onto the floor that exists now.
const wakeSleepers = () => {
  if (!props.size) return;
  for (const prop of props) prop.sleeping = false;
  ensureJuggleFrame();
};
addEventListener("resize", wakeSleepers);
visualViewport?.addEventListener("resize", wakeSleepers);

const stopJuggling = () => {
  if (juggleRaf) cancelAnimationFrame(juggleRaf);
  juggleRaf = 0;
  for (const prop of props) {
    if (
      prop.pointerId !== null &&
      prop.captureElement?.hasPointerCapture(prop.pointerId)
    ) {
      prop.captureElement.releasePointerCapture(prop.pointerId);
    }
    if (prop.originalStyle === null) prop.source.removeAttribute("style");
    else prop.source.setAttribute("style", prop.originalStyle);
  }
  props.clear();
  propFor = new WeakMap();
  juggleOverlay?.remove();
  juggleOverlay = null;
};

document.addEventListener(
  "pointerdown",
  (event) => {
    if (
      !active ||
      matchMedia("(prefers-reduced-motion: reduce)").matches ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.altKey ||
      event.shiftKey
    ) {
      return;
    }
    const target = juggleTarget(event.target);
    if (!target) return;
    event.preventDefault();
    event.stopPropagation();
    const prop = propFor.get(target) || makeProp(target);
    const box = prop.element.getBoundingClientRect();
    prop.held = true;
    prop.sleeping = false;
    prop.pointerId = event.pointerId;
    prop.captureElement = target;
    prop.grabX = event.clientX - box.left;
    prop.grabY = event.clientY - box.top;
    prop.lastX = event.clientX;
    prop.lastY = event.clientY;
    prop.lastAt = event.timeStamp;
    prop.vx = 0;
    prop.vy = 0;
    prop.dragged = false;
    prop.element.style.cursor = "grabbing";
    target.setPointerCapture(event.pointerId);
    suppressClicks.delete(target);
    ensureJuggleFrame();
  },
  true
);

document.addEventListener(
  "pointermove",
  (event) => {
    const target = juggleTarget(event.target);
    const prop = target && propFor.get(target);
    if (!prop || !prop.held || prop.pointerId !== event.pointerId) return;
    event.preventDefault();
    const dt = Math.max(8, event.timeStamp - prop.lastAt) / 1000;
    const dx = event.clientX - prop.lastX;
    const dy = event.clientY - prop.lastY;
    prop.x = event.clientX - prop.grabX;
    prop.y = event.clientY - prop.grabY;
    prop.vx = clampSpeed(prop.vx * 0.35 + (dx / dt) * 0.65);
    prop.vy = clampSpeed(prop.vy * 0.35 + (dy / dt) * 0.65);
    prop.spin = clampSpeed(prop.vx * 0.09);
    prop.dragged ||= Math.hypot(dx, dy) > 2;
    prop.lastX = event.clientX;
    prop.lastY = event.clientY;
    prop.lastAt = event.timeStamp;
    paintProp(prop);
  },
  true
);

const releaseProp = (event) => {
  const target = juggleTarget(event.target);
  const prop = target && propFor.get(target);
  if (!prop || !prop.held || prop.pointerId !== event.pointerId) return;
  event.preventDefault();
  event.stopPropagation();
  prop.held = false;
  prop.pointerId = null;
  prop.captureElement = null;
  prop.element.style.cursor = "grab";
  if (!prop.dragged && event.type === "pointerup") {
    const side = prop.grabX / Math.max(1, prop.width) - 0.5;
    prop.vx = side * 700 + (Math.random() - 0.5) * 180;
    prop.vy = -720;
    prop.spin = prop.vx * 0.12;
  }
  if (event.type === "pointerup") {
    suppressClicks.add(target);
    setTimeout(() => suppressClicks.delete(target), 0);
  }
  ensureJuggleFrame();
};

document.addEventListener("pointerup", releaseProp, true);
document.addEventListener("pointercancel", releaseProp, true);

document.addEventListener(
  "click",
  (event) => {
    const element = juggleTarget(event.target);
    if (!element || !suppressClicks.has(element)) return;
    suppressClicks.delete(element);
    event.preventDefault();
    event.stopImmediatePropagation();
  },
  true
);

export function activate() {
  active = true;
}

export function deactivate() {
  active = false;
  stopJuggling();
}

export function selected({ reducedMotion }) {
  celebrate(reducedMotion());
}
