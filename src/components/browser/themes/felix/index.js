// Felix's theme-package adapter and cursor chase. Loaded only when selected.

export const id = "felix";
export const colorScheme = "light";
export const music = Object.freeze({
  kind: "sequence",
  bpm: 112,
  wave: "triangle",
  bass: [
    36, null, 43, null, 36, null, 43, null, 41, null, 45, null, 43, null, 43,
    null,
  ],
  lead: [
    64, null, 67, null, 72, null, 67, null, 69, null, 65, null, 67, null,
    null, null,
  ],
  perc: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
});

export function tone({ tone }) {
  tone({ f: 1150, d: 0.14, w: "sine", v: 0.6, glide: 1500 });
  tone({ f: 1500, t: 0.14, d: 0.1, w: "sine", v: 0.5, glide: 1050 });
  tone({ f: 1200, t: 0.32, d: 0.12, w: "sine", v: 0.55, glide: 1550 });
  tone({ f: 1550, t: 0.44, d: 0.09, w: "sine", v: 0.45, glide: 1100 });
}

const catchSqueak = ({ tone }) => {
  const first = tone({ f: 1250, d: 0.12, w: "sine", v: 0.25, glide: 1600 });
  tone({ f: 1600, t: 0.12, d: 0.08, w: "sine", v: 0.2, glide: 1150 });
  return first;
};

const MOUTH_X = 0.29;
const MOUTH_Y = 0.32;
let chaser = null;
let chaserSrc = null;
let sound = null;
let chaseRaf = 0;
let px = 0;
let py = 0;
let mx = 0;
let my = 0;
let lastMove = 0;
let caught = false;
let lastSqueak = 0;
let lastFrame = 0;

const onMouse = (event) => {
  mx = event.clientX;
  my = event.clientY;
  lastMove = performance.now();
};

const chaseFrame = (now) => {
  chaseRaf = requestAnimationFrame(chaseFrame);
  const dt = Math.min(0.05, (now - lastFrame) / 1000 || 0.016);
  lastFrame = now;
  const gain = 1 - Math.exp(-3.4 * dt);
  const dx = mx - px;
  const dy = my - py;
  const dist = Math.hypot(dx, dy);
  const idle = now - lastMove;
  if (caught) {
    px = mx;
    py = my;
    if (dist > 1 && idle < 80) caught = false;
  } else {
    px += dx * gain;
    py += dy * gain;
    if (dist < 14 && idle > 250) {
      caught = true;
      if (sound && now - lastSqueak > 8000 && catchSqueak(sound)) {
        lastSqueak = now;
      }
    }
  }
  const width = chaser.offsetWidth || 70;
  const height = chaser.offsetHeight || 130;
  const moving = !caught && dist > 6;
  const bob = moving ? Math.sin(now * 0.02) * 3 : 0;
  const tilt = caught ? -3 : Math.max(-10, Math.min(10, dx * 0.02));
  chaser.style.transform =
    `translate(${px - width * MOUTH_X}px, ${py - height * MOUTH_Y + bob}px) ` +
    `rotate(${tilt}deg)`;
};

const startChase = () => {
  if (chaseRaf) return;
  if (!chaser) {
    chaser = document.createElement("img");
    chaser.alt = "";
    chaser.className = "felix-chaser";
    document.body.appendChild(chaser);
  }
  if (chaser.src !== chaserSrc) chaser.src = chaserSrc;
  chaser.hidden = false;
  mx = innerWidth / 2;
  my = innerHeight / 2;
  px = -100;
  py = innerHeight + 100;
  caught = false;
  addEventListener("mousemove", onMouse);
  chaseRaf = requestAnimationFrame(chaseFrame);
};

export function deactivate() {
  if (chaseRaf) cancelAnimationFrame(chaseRaf);
  chaseRaf = 0;
  removeEventListener("mousemove", onMouse);
  if (chaser) chaser.hidden = true;
}

export function activate({ assets, reducedMotion, sound: themeSound }) {
  chaserSrc = assets.image;
  sound = themeSound;
  const wants =
    chaserSrc && !reducedMotion() && matchMedia("(pointer: fine)").matches;
  if (wants) startChase();
  else deactivate();
}

export function selected() {}
