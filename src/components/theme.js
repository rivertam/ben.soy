// The theme switcher's interactions. The boot script in <head> (see
// content/themes.rs) already applied the stored theme before first paint;
// this file handles clicks, keeps aria-pressed true on the worn theme,
// mirrors changes across tabs, and carries the whole audio department:
// a synthesized signature sting per theme and an opt-in 16-step tune,
// all WebAudio oscillators (no audio assets, nothing plays without a
// user gesture). No Tailwind class strings in here: the scanner only
// reads .rs files, so styling stays server-rendered; confetti uses
// inline styles only.
(() => {
  const KEY = "bens-theme";
  const MUSIC_KEY = "bens-theme-music";
  // The id worn when <html> carries no data-theme: an explicit choice of
  // the house finish. The site's DEFAULT is tmux — chrome.rs SSRs it on
  // <html>, and the boot script leaves it standing unless storage says
  // otherwise — so bare is a choice here, not the starting state.
  const BARE_ID = "oxide";
  const root = document.documentElement;
  const wornId = () => root.dataset.theme || BARE_ID;
  const reducedMotion = () =>
    matchMedia("(prefers-reduced-motion: reduce)").matches;
  // A page that renders [data-page-band] (an <audio> like /podrick's
  // anthem) brought its own music; the conductor drives it instead of a
  // theme tune.
  const band = () => document.querySelector("[data-page-band]");
  // One master volume for everything audible. 0.5 is the designed level;
  // each source scales from its own base so the balance holds.
  const VOLUME_KEY = "bens-theme-volume";
  let vol = 0.5;
  try {
    const stored = parseFloat(localStorage.getItem(VOLUME_KEY));
    if (Number.isFinite(stored)) vol = Math.min(1, Math.max(0, stored));
  } catch {
    // No storage: the designed level stands.
  }
  const applyVolume = () => {
    if (stingBus) stingBus.gain.value = 0.11 * vol;
    if (musicBus) musicBus.gain.value = 0.09 * vol;
    if (tuneEl) tuneEl.volume = Math.min(1, 0.7 * vol);
    const b = band();
    if (b) b.volume = Math.min(1, 1.2 * vol);
  };

  /* ── audio engine ─────────────────────────────────────────────────── */
  // One context for everything, created on first gesture and reused.
  let ctx = null;
  let stingBus = null;
  let musicBus = null;
  const audio = () => {
    if (!ctx) {
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) return null;
      ctx = new AC();
      stingBus = ctx.createGain();
      stingBus.connect(ctx.destination);
      musicBus = ctx.createGain();
      musicBus.connect(ctx.destination);
      applyVolume();
    }
    if (ctx.state === "suspended") ctx.resume();
    return ctx;
  };

  // One note: oscillator through a percussive envelope.
  const tone = (bus, { f, t = 0, d = 0.15, w = "sine", v = 1, glide }) => {
    const at = ctx.currentTime + t;
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = w;
    osc.frequency.setValueAtTime(f, at);
    if (glide) osc.frequency.exponentialRampToValueAtTime(glide, at + d);
    gain.gain.setValueAtTime(v, at);
    gain.gain.exponentialRampToValueAtTime(0.001, at + d);
    osc.connect(gain).connect(bus);
    osc.start(at);
    osc.stop(at + d + 0.02);
  };

  // One thwack of filtered noise (percussion, pencils, steam).
  const noise = (bus, { t = 0, d = 0.08, f = 2000, q = 1, v = 1 }) => {
    const at = ctx.currentTime + t;
    const len = Math.ceil(ctx.sampleRate * d);
    const buffer = ctx.createBuffer(1, len, ctx.sampleRate);
    const data = buffer.getChannelData(0);
    for (let i = 0; i < len; i++) data[i] = Math.random() * 2 - 1;
    const src = ctx.createBufferSource();
    src.buffer = buffer;
    const filter = ctx.createBiquadFilter();
    filter.type = "bandpass";
    filter.frequency.value = f;
    filter.Q.value = q;
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(v, at);
    gain.gain.exponentialRampToValueAtTime(0.001, at + d);
    src.connect(filter).connect(gain).connect(bus);
    src.start(at);
  };

  /* ── signature stings, one per theme ──────────────────────────────── */
  const STINGS = {
    // The mill: one clean hammer strike on cooling steel.
    oxide: () => {
      noise(stingBus, { d: 0.12, f: 2600, q: 4, v: 0.9 });
      tone(stingBus, { f: 220, d: 0.3, w: "triangle", v: 0.7, glide: 180 });
    },
    // Night shift: the furnace draft catching.
    dark: () => {
      noise(stingBus, { d: 0.5, f: 400, q: 0.7, v: 0.6 });
      tone(stingBus, { f: 90, d: 0.5, w: "sine", v: 0.5, glide: 60 });
    },
    // tmux: one keypress, then the attach — a rising double bell.
    tmux: () => {
      noise(stingBus, { d: 0.05, f: 3200, q: 2, v: 0.5 });
      tone(stingBus, { f: 660, t: 0.07, d: 0.09, w: "square", v: 0.32 });
      tone(stingBus, { f: 990, t: 0.18, d: 0.14, w: "square", v: 0.28 });
    },
    // Felix: the squeaky toy, twice, because once was incredible.
    felix: () => {
      tone(stingBus, { f: 1150, d: 0.14, w: "sine", v: 0.6, glide: 1500 });
      tone(stingBus, { f: 1500, t: 0.14, d: 0.1, w: "sine", v: 0.5, glide: 1050 });
      tone(stingBus, { f: 1200, t: 0.32, d: 0.12, w: "sine", v: 0.55, glide: 1550 });
      tone(stingBus, { f: 1550, t: 0.44, d: 0.09, w: "sine", v: 0.45, glide: 1100 });
    },
    // Clown: slide whistle up, then the nose, twice. We are not sorry.
    clown: () => {
      tone(stingBus, { f: 400, d: 0.32, w: "sine", v: 0.5, glide: 950 });
      tone(stingBus, { f: 233, t: 0.4, d: 0.16, w: "square", v: 0.5, glide: 175 });
      tone(stingBus, { f: 233, t: 0.62, d: 0.18, w: "square", v: 0.5, glide: 170 });
    },
  };

  /* ── theme tunes: only the especially whimsical carry one ───────── */
  // Sixteenth-step patterns; numbers are MIDI notes, null is a rest. The
  // registry marks felix and clown as especially whimsical (the 🎪 in
  // the menu); constant page sound is their perk alone. Everyone else
  // keeps a one-shot selection sting and their dignity. Clown outgrew the
  // synth: it hires the real band below (an mp3 whose hashed URL rides
  // the ♪ row's data-clown-tune attribute).
  const TUNES = {
    felix: {
      bpm: 112,
      wave: "triangle",
      bass: [36, null, 43, null, 36, null, 43, null, 41, null, 45, null, 43, null, 43, null],
      lead: [64, null, 67, null, 72, null, 67, null, 69, null, 65, null, 67, null, null, null],
      perc: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    },
  };

  // The mp3 track for a theme, if it has one (only clown today).
  const tuneSrcFor = (id) =>
    id === "clown"
      ? document.querySelector("[data-clown-tune]")?.dataset.clownTune
      : undefined;

  const midi = (m) => 440 * Math.pow(2, (m - 69) / 12);

  // A lookahead scheduler: a slow interval books notes slightly ahead on
  // the audio clock, so tab jank never slurs the tune. Mp3 themes bypass
  // it for a looping <audio> element instead.
  let musicOn = false;
  let step = 0;
  let nextAt = 0;
  let timer = null;
  let mp3Timer = null;
  let tuneEl = null;
  // The theme currently sounding (either engine), so switches restart the
  // right music and everything else is a no-op.
  let playing = null;
  // Fresh starts normally begin almost at once; a themed selection sets
  // this higher so the signature sting gets the downbeat to itself.
  let startDelay = 0.05;
  const scheduleAhead = () => {
    const tune = TUNES[wornId()];
    if (!tune) return;
    const stepDur = 60 / tune.bpm / 4;
    while (nextAt < ctx.currentTime + 0.25) {
      const t = nextAt - ctx.currentTime;
      const i = step % 16;
      if (tune.bass[i] !== null)
        tone(musicBus, { f: midi(tune.bass[i]), t, d: stepDur * 1.8, w: tune.wave, v: 0.5 });
      if (tune.lead[i] !== null)
        tone(musicBus, { f: midi(tune.lead[i]), t, d: stepDur * 2.6, w: tune.wave, v: 0.3 });
      if (tune.perc[i])
        noise(musicBus, { t, d: 0.025, f: 4000, q: 1.5, v: 0.4 });
      nextAt += stepDur;
      step += 1;
    }
  };
  const startMusic = () => {
    if (!audio()) return;
    const worn = wornId();
    if (playing === worn) return;
    stopMusic();
    playing = worn;
    const delay = startDelay;
    startDelay = 0.05;
    const src = tuneSrcFor(worn);
    if (src) {
      if (!tuneEl) {
        tuneEl = new Audio();
        tuneEl.loop = true;
      }
      applyVolume();
      if (!tuneEl.src.endsWith(src)) tuneEl.src = src;
      // No currentTime reset: a hidden-tab pause resumes where it left
      // off, and re-entering the tent mid-song is part of the bit.
      mp3Timer = setTimeout(() => {
        tuneEl.play().catch(() => {
          // Autoplay said no (no activation yet): the armed first-gesture
          // listener or the next toggle press will try again.
          playing = null;
        });
      }, delay * 1000);
    } else {
      step = 0;
      nextAt = ctx.currentTime + delay;
      scheduleAhead();
      timer = setInterval(scheduleAhead, 100);
    }
  };
  const stopMusic = () => {
    if (timer) clearInterval(timer);
    timer = null;
    if (mp3Timer) clearTimeout(mp3Timer);
    mp3Timer = null;
    if (tuneEl) tuneEl.pause();
    playing = null;
  };
  // The tune runs only when: the toggle is on, the worn theme is one of
  // the whimsical two, the tab is visible, and a gesture has unlocked
  // audio. Called at every point any of those change.
  const syncMusic = () => {
    const b = band();
    if (b) {
      // The page's own band plays (element playback needs no
      // AudioContext); theme tunes stay off its stage.
      stopMusic();
      if (musicOn && !document.hidden) {
        applyVolume();
        b.play().catch(() => {
          // Autoplay veto: the armed first gesture retries via syncMusic.
        });
      } else {
        b.pause();
      }
    } else if (ctx) {
      const worn = wornId();
      const carries = TUNES[worn] || tuneSrcFor(worn);
      if (musicOn && carries && !document.hidden) startMusic();
      else stopMusic();
    }
    syncTransport();
  };

  // The corner transport: visible whenever this page has a source to
  // control, glyph from what is actually sounding right now.
  const syncTransport = () => {
    const wrap = document.querySelector(".band-wrap");
    const pill = document.querySelector(".band-pill");
    if (!wrap || !pill) return;
    const b = band();
    const worn = wornId();
    const source = b || TUNES[worn] || tuneSrcFor(worn);
    wrap.hidden = !source;
    const sounding = b ? !b.paused && !b.ended : !!playing;
    pill.textContent = sounding ? "\u23F8" : "\u25B6";
  };

  const setMusic = (on, { fromGesture = true } = {}) => {
    musicOn = on;
    document
      .querySelectorAll("[data-music-toggle]")
      .forEach((b) => b.setAttribute("aria-pressed", String(on)));
    if (fromGesture) {
      // Only a real toggle press persists the choice; boot and cross-tab
      // syncing must not overwrite it.
      try {
        localStorage.setItem(MUSIC_KEY, on ? "on" : "off");
      } catch {
        // Private mode: the tune still plays for this page.
      }
      audio();
    }
    syncMusic();
  };

  // Politeness: no tune for a hidden tab.
  document.addEventListener("visibilitychange", () => syncMusic());

  /* ── confetti (clown selection only) ──────────────────────────────── */
  const confetti = () => {
    if (reducedMotion() || !("animate" in Element.prototype)) return;
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

  /* ── juggling (clown mode) ───────────────────────────────────────── */
  // Links and loose images become little viewport-bound physics props. A
  // press picks one up; a quick click tosses it, while a drag uses the
  // pointer's release velocity. The moving prop is a pixel-matched copy in
  // a viewport overlay. Keeping the source invisibly in flow avoids inline
  // text reflow and transformed-ancestor coordinate systems moving the prop
  // away from the pointer. Theme controls stay bolted to the floor so there
  // is always a way out of the tent.
  const GRAVITY = 1800;
  const props = new Set();
  let propFor = new WeakMap();
  const suppressClicks = new WeakSet();
  let juggleRaf = 0;
  let juggleLastFrame = 0;
  let juggleOverlay = null;

  const juggleTarget = (target) => {
    if (!(target instanceof Element)) return null;
    if (target.closest(".corner-rack, [data-no-clown-physics]")) return null;
    const link = target.closest("a");
    if (link) return link;
    // Many pages present their most visual objects through buttons: Felix's
    // photos open a lightbox, and inline-popover labels look and read like
    // links. In clown mode those are props too; authors can keep a specific
    // control functional with data-no-clown-physics.
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
    // The overlay is fixed to the viewport; its children can therefore use
    // the same coordinates as PointerEvent.clientX/Y on every page.
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

      const right = Math.max(0, innerWidth - prop.width);
      const floor = Math.max(0, innerHeight - prop.height);
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
        // End the tiny asymptotic hops, but leave the prop on the floor to
        // be picked up and thrown again.
        if (Math.abs(prop.vy) < 70 && Math.abs(prop.vx) < 28) {
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
        wornId() !== "clown" ||
        reducedMotion() ||
        event.button !== 0 ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey ||
        event.shiftKey
      )
        return;
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
      // A click is a jaunty upward toss. Its horizontal direction depends
      // on which side was grabbed, so repeated clicks can keep it aloft.
      const side = prop.grabX / Math.max(1, prop.width) - 0.5;
      prop.vx = side * 700 + (Math.random() - 0.5) * 180;
      prop.vy = -720;
      prop.spin = prop.vx * 0.12;
    }
    if (event.type === "pointerup") {
      suppressClicks.add(target);
      // A prevented pointerdown does not produce a click in every browser.
      // Do not let a missing compatibility click poison the next keyboard
      // activation of the same link.
      setTimeout(() => suppressClicks.delete(target), 0);
    }
    ensureJuggleFrame();
  };
  document.addEventListener("pointerup", releaseProp, true);
  document.addEventListener("pointercancel", releaseProp, true);

  // Cancel only the synthetic click following a handled pointer gesture.
  // Keyboard activation and modifier-clicks retain ordinary navigation.
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

  /* ── the fetch (felix mode) ───────────────────────────────────────── */
  // Felix — cut out of the sprint photo — trails the tennis-ball cursor
  // with smooth pursuit, and when the ball rests he catches up and holds
  // it in his mouth until it moves again. Fine pointers only; the global
  // reduced-motion preference cancels the game entirely.
  const MOUTH_X = 0.29; // the open mouth, as fractions of the rendered img
  const MOUTH_Y = 0.32;
  let chaser = null;
  let chaseRaf = 0;
  let px = 0;
  let py = 0; // Felix's mouth point, in viewport coords
  let mx = 0;
  let my = 0; // the ball (cursor)
  let lastMove = 0;
  let caught = false;
  let lastSqueak = 0;
  const onMouse = (event) => {
    mx = event.clientX;
    my = event.clientY;
    lastMove = performance.now();
  };
  const chaserSrc = () =>
    document.querySelector("[data-felix-chaser]")?.dataset.felixChaser;

  let lastFrame = 0;
  const chaseFrame = (now) => {
    chaseRaf = requestAnimationFrame(chaseFrame);
    // Time-based smoothing so the chase takes the same real seconds at
    // any frame rate (clamped so an occluded tab can't teleport him).
    const dt = Math.min(0.05, (now - lastFrame) / 1000 || 0.016);
    lastFrame = now;
    const gain = 1 - Math.exp(-3.4 * dt);
    const dx = mx - px;
    const dy = my - py;
    const dist = Math.hypot(dx, dy);
    const idle = now - lastMove;
    if (caught) {
      // He has the ball. A big enough yank starts the game again.
      px = mx;
      py = my;
      if (dist > 1 && idle < 80) caught = false;
    } else {
      px += dx * gain;
      py += dy * gain;
      if (dist < 14 && idle > 250) {
        caught = true;
        // One quiet victory squeak, at most every eight seconds, and only
        // if a gesture already unlocked audio.
        if (ctx && now - lastSqueak > 8000) {
          lastSqueak = now;
          tone(stingBus, { f: 1250, d: 0.12, w: "sine", v: 0.25, glide: 1600 });
          tone(stingBus, { f: 1600, t: 0.12, d: 0.08, w: "sine", v: 0.2, glide: 1150 });
        }
      }
    }
    const w = chaser.offsetWidth || 70;
    const h = chaser.offsetHeight || 130;
    // Running gait while mid-chase; a lean into the turn from velocity.
    const moving = !caught && dist > 6;
    const bob = moving ? Math.sin(now * 0.02) * 3 : 0;
    const tilt = caught ? -3 : Math.max(-10, Math.min(10, dx * 0.02));
    chaser.style.transform = `translate(${px - w * MOUTH_X}px, ${py - h * MOUTH_Y + bob}px) rotate(${tilt}deg)`;
  };

  const startChase = () => {
    if (chaseRaf) return;
    if (!chaser) {
      chaser = document.createElement("img");
      chaser.src = chaserSrc();
      chaser.alt = "";
      chaser.className = "felix-chaser";
      document.body.appendChild(chaser);
    }
    chaser.hidden = false;
    // He enters from just off the bottom-left, wherever the ball is.
    mx = innerWidth / 2;
    my = innerHeight / 2;
    px = -100;
    py = innerHeight + 100;
    caught = false;
    addEventListener("mousemove", onMouse);
    chaseRaf = requestAnimationFrame(chaseFrame);
  };
  const stopChase = () => {
    if (!chaseRaf) return;
    cancelAnimationFrame(chaseRaf);
    chaseRaf = 0;
    removeEventListener("mousemove", onMouse);
    if (chaser) chaser.hidden = true;
  };
  const syncChaser = () => {
    const wants =
      wornId() === "felix" &&
      !reducedMotion() &&
      matchMedia("(pointer: fine)").matches &&
      chaserSrc();
    if (wants) startChase();
    else stopChase();
  };

  /* ── tmux mode: the site is a session you're attached to ──────────── */
  // chrome.rs renders the status bar (session badge, numbered windows —
  // the same destinations as the ~ listing — and a clock); themes.css
  // reveals it only under [data-theme="tmux"]. This section is the keys:
  // ctrl-a is the prefix (n/p cycle windows, 0-9 jump, l bounces to the
  // last one), j/k walk the log and the ~ listing under a cursorline
  // (plain scrolling anywhere else), gg/G reach the ends, Enter opens the
  // selected entry, and f sprays vimium-style hints over everything
  // interactable. All of it no-ops unless tmux is the worn theme.
  const TMUX_LAST_KEY = "bens-tmux-last";
  const tmuxOn = () => wornId() === "tmux";
  const tmuxBar = () => document.querySelector("[data-tmux-bar]");
  const tmuxWindows = () => [
    ...document.querySelectorAll("[data-tmux-window]"),
  ];

  let tmuxArmed = false;
  let tmuxArmTimer = null;
  const armPrefix = (on) => {
    tmuxArmed = on;
    clearTimeout(tmuxArmTimer);
    tmuxArmTimer = null;
    const bar = tmuxBar();
    if (on) {
      if (bar) bar.dataset.prefixArmed = "";
      tmuxArmTimer = setTimeout(() => armPrefix(false), 2000);
    } else if (bar) {
      delete bar.dataset.prefixArmed;
    }
  };

  // Window switches note where they left from so prefix-l can bounce
  // between two windows like tmux's last-window. Per tab on purpose:
  // each tab is its own attached client.
  const tmuxGo = (href) => {
    try {
      sessionStorage.setItem(
        TMUX_LAST_KEY,
        location.pathname + location.search
      );
    } catch {
      // Private mode: l just has nowhere to bounce back to.
    }
    location.href = href;
  };
  const tmuxCycle = (delta) => {
    const wins = tmuxWindows();
    if (!wins.length) return;
    const at = Math.max(
      0,
      wins.findIndex((win) => win.hasAttribute("aria-current"))
    );
    tmuxGo(wins[(at + delta + wins.length) % wins.length].getAttribute("href"));
  };
  const tmuxJump = (digit) => {
    const win = tmuxWindows()[digit];
    if (win) tmuxGo(win.getAttribute("href"));
  };
  const tmuxLastWindow = () => {
    let last = null;
    try {
      last = sessionStorage.getItem(TMUX_LAST_KEY);
    } catch {
      // No storage, no bounce.
    }
    if (last && last !== location.pathname + location.search) tmuxGo(last);
  };

  // j/k: a cursorline over the log's timeline and the ~ listing's rows.
  // Year badges are skipped — they're furniture, not entries.
  let logRows = null;
  let logAt = -1;
  const logEntries = () => {
    if (!logRows) {
      logRows = [
        ...document.querySelectorAll(".log-timeline .log-row, .netrw .netrw-row"),
      ].filter((row) => !row.querySelector(".log-year"));
    }
    return logRows;
  };
  const logClear = () => {
    for (const row of logEntries()) row.classList.remove("tmux-cursorline");
    logAt = -1;
  };
  const logSelect = (index) => {
    const rows = logEntries();
    if (!rows.length) return false;
    logAt = Math.max(0, Math.min(rows.length - 1, index));
    rows.forEach((row, i) =>
      row.classList.toggle("tmux-cursorline", i === logAt)
    );
    rows[logAt].scrollIntoView({ block: "nearest" });
    return true;
  };
  const logMove = (delta) => {
    const rows = logEntries();
    if (!rows.length) return false;
    if (logAt < 0) return logSelect(delta > 0 ? 0 : rows.length - 1);
    return logSelect(logAt + delta);
  };

  // f: vimium-style hints. Fixed-length labels over the home row, sized
  // so every visible interactable gets a unique tag.
  const HINT_CHARS = "sadfjklewcmpgh";
  let hintState = null;
  const hintLabels = (count) => {
    let len = 1;
    while (Math.pow(HINT_CHARS.length, len) < count) len += 1;
    const labels = [];
    const grow = (prefix) => {
      if (labels.length >= count) return;
      if (prefix.length === len) {
        labels.push(prefix);
        return;
      }
      for (const ch of HINT_CHARS) grow(prefix + ch);
    };
    grow("");
    return labels;
  };
  const hintTargets = () =>
    [
      ...document.querySelectorAll(
        "a[href], button, summary, input, select, textarea, [role='button'], audio[controls]"
      ),
    ].filter((el) => {
      if (el.disabled || el.closest("[hidden]")) return false;
      const box = el.getBoundingClientRect();
      return (
        box.width > 1 &&
        box.height > 1 &&
        box.bottom > 0 &&
        box.right > 0 &&
        box.top < innerHeight &&
        box.left < innerWidth &&
        getComputedStyle(el).visibility !== "hidden"
      );
    });
  const endHints = () => {
    if (!hintState) return;
    for (const overlay of hintState.overlays) overlay.remove();
    hintState = null;
    removeEventListener("resize", endHints);
  };
  // The open [popover] an element lives in, if any. Popover panels sit in
  // the browser's top layer, which paints over every z-index — a chip for a
  // target in there is only visible if it rides inside the same panel.
  const openPopoverOf = (el) => {
    const pop = el.closest("[popover]");
    try {
      return pop && pop.matches(":popover-open") ? pop : null;
    } catch {
      // No Popover API: panels render in-flow and the page overlay serves.
      return null;
    }
  };
  const paintHints = () => {
    for (const { chip, label } of hintState.chips) {
      const live = label.startsWith(hintState.typed);
      chip.style.display = live ? "" : "none";
      for (let i = 0; i < chip.children.length; i++) {
        chip.children[i].classList.toggle(
          "tmux-hint-typed",
          live && i < hintState.typed.length
        );
      }
    }
  };
  const startHints = () => {
    endHints();
    const targets = hintTargets();
    if (!targets.length) return;
    const labels = hintLabels(targets.length);
    // One overlay per hosting surface: <body> for the page, plus each open
    // popover that contains a target. Chips are positioned relative to
    // their overlay, so the page's stay glued while scrolling (the way
    // vimium does it) and a popover's ride along with its panel.
    const overlays = new Map();
    const overlayIn = (host) => {
      let entry = overlays.get(host);
      if (!entry) {
        const overlay = document.createElement("div");
        overlay.className = "tmux-hints";
        overlay.setAttribute("aria-hidden", "true");
        host.appendChild(overlay);
        entry = { overlay, origin: overlay.getBoundingClientRect() };
        overlays.set(host, entry);
      }
      return entry;
    };
    const chips = targets.map((target, i) => {
      const { overlay, origin } = overlayIn(
        openPopoverOf(target) || document.body
      );
      const box = target.getBoundingClientRect();
      const chip = document.createElement("span");
      chip.className = "tmux-hint";
      for (const ch of labels[i]) {
        const key = document.createElement("span");
        key.textContent = ch;
        chip.appendChild(key);
      }
      chip.style.top = `${Math.max(2, box.top - origin.top - 8)}px`;
      chip.style.left = `${Math.max(2, box.left - origin.left - 6)}px`;
      overlay.appendChild(chip);
      return { chip, target, label: labels[i] };
    });
    hintState = {
      overlays: [...overlays.values()].map(({ overlay }) => overlay),
      chips,
      typed: "",
    };
    // A resize does move the anchors out from under the chips: fold.
    addEventListener("resize", endHints);
  };
  const fireHint = (target) => {
    endHints();
    if (target.matches("input, textarea, select")) target.focus();
    else target.click();
  };
  const hintKey = (event) => {
    if (event.ctrlKey || event.altKey || event.metaKey) {
      endHints();
      return;
    }
    if (event.key === "Shift") return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      endHints();
      return;
    }
    if (event.key === "Backspace") {
      hintState.typed = hintState.typed.slice(0, -1);
      paintHints();
      return;
    }
    const key = event.key.toLowerCase();
    if (key.length !== 1 || !HINT_CHARS.includes(key)) {
      endHints();
      return;
    }
    const next = hintState.typed + key;
    const hit = hintState.chips.find(({ label }) => label === next);
    if (hit) {
      fireHint(hit.target);
      return;
    }
    if (!hintState.chips.some(({ label }) => label.startsWith(next))) return;
    hintState.typed = next;
    paintHints();
  };

  // display-message: a transient note across the status line, tmux's own
  // way of talking. Click dismisses; so does leaving the theme.
  let noteTimer = null;
  const tmuxMessage = (text) => {
    const bar = tmuxBar();
    if (!bar) return;
    let note = bar.querySelector(".tmux-note");
    if (!note) {
      note = document.createElement("span");
      note.className = "tmux-note";
      note.setAttribute("role", "status");
      note.title = "dismiss";
      note.addEventListener("click", () => note.remove());
      bar.appendChild(note);
    }
    note.textContent = text;
    clearTimeout(noteTimer);
    noteTimer = setTimeout(() => note.remove(), 8000);
  };

  // Vimium fights this theme for f and j/k (it swallows the keys before
  // the page ever sees them). Its web-accessible resources answer a
  // fetch whenever the extension is *installed* — including after a
  // per-site exclusion — so the probe alone can't tell active from idle.
  // We only nag when install looks likely, overlapping keys have not
  // yet reached the page (proof the exclusion stuck), and the viewer
  // hasn't already been told. Noted/ok live in localStorage so a
  // finished exclusion survives new tabs; DOM markers still catch forks
  // / live HUD the id probe can't cover.
  const VIMIUM_NOTED_KEY = "bens-tmux-vimium-noted";
  const VIMIUM_OK_KEY = "bens-tmux-vimium-ok";
  const VIMIUM_STORE_ID = "dbepggeogbaibhgnhhndojpepiihcmeb";
  let vimiumTimer = null;
  const vimiumStore = (key, value) => {
    try {
      if (value === undefined) return localStorage.getItem(key);
      localStorage.setItem(key, value);
    } catch {
      return null;
    }
  };
  // Older builds stored the ack per tab; promote it so a refresh right
  // after an exclusion doesn't re-nag.
  try {
    if (sessionStorage.getItem(VIMIUM_NOTED_KEY) === "yes") {
      vimiumStore(VIMIUM_NOTED_KEY, "yes");
    }
  } catch {
    // No sessionStorage: nothing to promote.
  }
  const markVimiumOk = () => {
    // j/k/f reached us ⇒ Vimium isn't capturing on this origin.
    vimiumStore(VIMIUM_OK_KEY, "yes");
    if (vimiumTimer) {
      clearTimeout(vimiumTimer);
      vimiumTimer = null;
    }
    clearTimeout(noteTimer);
    noteTimer = null;
    document.querySelector(".tmux-note")?.remove();
  };
  const vimiumInstalled = async () => {
    if (document.querySelector('[class*="vimium" i], [id*="vimium" i]')) {
      return true;
    }
    if (!("chrome" in window)) return false;
    try {
      await fetch(
        `chrome-extension://${VIMIUM_STORE_ID}/content_scripts/vimium.css`,
        { mode: "no-cors" }
      );
      return true;
    } catch {
      // Not installed (or not Chrome): the fetch refuses.
      return false;
    }
  };
  const noteVimium = async () => {
    if (vimiumStore(VIMIUM_NOTED_KEY) === "yes") return;
    if (vimiumStore(VIMIUM_OK_KEY) === "yes") return;
    // tmuxOn / ok are rechecked after the await: the probe is async and
    // a key (or costume change) may have landed mid-flight.
    if (!(await vimiumInstalled()) || !tmuxOn()) return;
    if (vimiumStore(VIMIUM_OK_KEY) === "yes") return;
    tmuxMessage(
      "vimium detected. First of all, nice. Second, I basically inlined vimium on this theme, so try disabling it for this site. :)"
    );
    vimiumStore(VIMIUM_NOTED_KEY, "yes");
  };

  // The status-line clock, repainted often enough to never lie by more
  // than a blink.
  let tmuxClockTimer = null;
  const paintTmuxClock = () => {
    const clock = document.querySelector("[data-tmux-clock]");
    if (clock) clock.textContent = new Date().toTimeString().slice(0, 5);
  };
  const syncTmux = () => {
    if (tmuxOn()) {
      paintTmuxClock();
      if (!tmuxClockTimer) tmuxClockTimer = setInterval(paintTmuxClock, 20000);
      if (
        !vimiumTimer &&
        vimiumStore(VIMIUM_NOTED_KEY) !== "yes" &&
        vimiumStore(VIMIUM_OK_KEY) !== "yes"
      ) {
        vimiumTimer = setTimeout(() => {
          vimiumTimer = null;
          noteVimium();
        }, 1500);
      }
    } else {
      clearInterval(tmuxClockTimer);
      tmuxClockTimer = null;
      clearTimeout(vimiumTimer);
      vimiumTimer = null;
      clearTimeout(noteTimer);
      noteTimer = null;
      document.querySelector(".tmux-note")?.remove();
      armPrefix(false);
      endHints();
      logClear();
    }
  };

  let lastG = 0;
  document.addEventListener("keydown", (event) => {
    if (!tmuxOn()) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest("input, textarea, select, [contenteditable='true']")) {
      return; // never steal from a prompt
    }
    if (hintState) {
      hintKey(event);
      return;
    }
    if (
      event.ctrlKey &&
      !event.altKey &&
      !event.metaKey &&
      !event.shiftKey &&
      (event.key === "a" || event.key === "b")
    ) {
      // The prefix: ctrl-a like the home session, ctrl-b for stock tmux
      // hands. The browser reads ctrl-a as select-all; the session
      // outranks it.
      event.preventDefault();
      armPrefix(!tmuxArmed);
      return;
    }
    if (tmuxArmed) {
      armPrefix(false);
      if (event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.key === "n") tmuxCycle(1);
      else if (event.key === "p") tmuxCycle(-1);
      else if (event.key === "l") tmuxLastWindow();
      else if (/^[0-9]$/.test(event.key)) tmuxJump(Number(event.key));
      else return;
      event.preventDefault();
      return;
    }
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    switch (event.key) {
      case "j":
        // Explicitly instant: smooth scrolling under key-repeat queues
        // animations and feels like wading. Landing here also means
        // Vimium isn't eating the key on this origin.
        markVimiumOk();
        if (!logMove(1)) scrollBy({ top: 80, behavior: "instant" });
        event.preventDefault();
        break;
      case "k":
        markVimiumOk();
        if (!logMove(-1)) scrollBy({ top: -80, behavior: "instant" });
        event.preventDefault();
        break;
      case "f":
        markVimiumOk();
        startHints();
        event.preventDefault();
        break;
      case "G":
        if (!logSelect(logEntries().length - 1)) {
          scrollTo({
            top: document.documentElement.scrollHeight,
            behavior: "instant",
          });
        }
        event.preventDefault();
        break;
      case "g":
        if (performance.now() - lastG < 450) {
          if (!logSelect(0)) scrollTo({ top: 0, behavior: "instant" });
        }
        lastG = performance.now();
        break;
      case "Enter": {
        if (logAt < 0) return;
        // A focused control keeps its own Enter.
        if (target?.closest("a, button, summary, [tabindex]")) return;
        const link = logEntries()[logAt]?.querySelector("a");
        if (link) {
          event.preventDefault();
          link.click();
        }
        break;
      }
      case "Escape":
        logClear();
        break;
    }
  });

  /* ── theme switching ──────────────────────────────────────────────── */
  const apply = (id) => {
    if (!id || id === BARE_ID) delete root.dataset.theme;
    else root.dataset.theme = id;
    const worn = wornId();
    if (worn !== "clown") stopJuggling();
    for (const button of document.querySelectorAll("[data-set-theme]")) {
      button.setAttribute("aria-pressed", String(button.dataset.setTheme === worn));
    }
    syncMusic();
    syncChaser();
    syncTmux();
  };

  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (!target) return;

    const musicButton = target.closest("[data-music-toggle]");
    if (musicButton) {
      const b = band();
      const sounding = b ? !b.paused && !b.ended : !!playing;
      // Pause only what is actually sounding; a click on a silent source
      // (vetoed autoplay, an ended anthem) starts it over instead of
      // silently flipping the remembered preference to off.
      audio();
      setMusic(!(musicOn && sounding));
      return;
    }

    const button = target.closest("[data-set-theme]");
    if (button) {
      const id = button.dataset.setTheme;
      // Sting first: it owns the first second, and the tune (if this theme
      // carries one and music wasn't turned off) comes in after it.
      if (audio() && STINGS[id]) {
        STINGS[id]();
        if (musicOn && (TUNES[id] || tuneSrcFor(id)) && playing !== id) {
          startDelay = 1.0;
        }
      }
      apply(id);
      try {
        localStorage.setItem(KEY, id);
      } catch {
        // Private-mode storage failure: the theme still applies.
      }
      if (id === "clown") confetti();
      button.closest("details")?.removeAttribute("open");
      return;
    }
    // Click-away closes the floating menu (a plain CSS-only <details>
    // would stay open; this one floats over content, so it earns the
    // courtesy).
    const open = document.querySelector(".theme-dd[open]");
    if (open && !open.contains(target)) open.removeAttribute("open");
  });

  // The volume slider (in the corner transport) scales every source live.
  document.addEventListener("input", (event) => {
    const slider =
      event.target instanceof Element && event.target.closest(".band-volume");
    if (!slider) return;
    vol = Math.min(1, Math.max(0, slider.value / 100));
    try {
      localStorage.setItem(VOLUME_KEY, String(vol));
    } catch {
      // Private mode: the level still applies for this page.
    }
    applyVolume();
  });

  // Another tab changed its mind: follow the theme, and follow the music
  // preference too — with the tune on by default, two tabs would otherwise
  // both be playing it.
  addEventListener("storage", (event) => {
    if (event.key === KEY) apply(event.newValue);
    if (event.key === MUSIC_KEY) {
      setMusic(event.newValue !== "off", { fromGesture: false });
    }
  });

  // Sync aria-pressed with whatever the boot script chose. The tune is ON
  // by default for the themes that carry one — the ♪ row is the opt-out,
  // and only an explicit "off" is remembered against it. Autoplay rules
  // mean a reload can't resume sound by itself, so arm the first gesture.
  const bootSlider = document.querySelector(".band-volume");
  if (bootSlider) bootSlider.value = String(Math.round(vol * 100));
  {
    const b = band();
    if (b) {
      for (const kind of ["play", "pause", "ended"]) {
        b.addEventListener(kind, syncTransport);
      }
    }
  }
  apply(wornId());
  let wantsMusic = true;
  try {
    wantsMusic = localStorage.getItem(MUSIC_KEY) !== "off";
  } catch {
    // No storage: default stands.
  }
  setMusic(wantsMusic, { fromGesture: false });
  if (wantsMusic) {
    const arm = () => {
      if (musicOn && audio()) syncMusic();
    };
    addEventListener("pointerdown", arm, { once: true });
    addEventListener("keydown", arm, { once: true });
  }
})();
