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
  const DEFAULT_ID = "oxide";
  const root = document.documentElement;
  const wornId = () => root.dataset.theme || DEFAULT_ID;
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
    // Terminal: degauss thump, then the tube warms up.
    terminal: () => {
      tone(stingBus, { f: 55, d: 0.12, w: "square", v: 0.5 });
      tone(stingBus, { f: 220, t: 0.1, d: 0.22, w: "square", v: 0.35, glide: 880 });
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

  /* ── theme switching ──────────────────────────────────────────────── */
  const apply = (id) => {
    if (!id || id === DEFAULT_ID) delete root.dataset.theme;
    else root.dataset.theme = id;
    const worn = wornId();
    for (const button of document.querySelectorAll("[data-set-theme]")) {
      button.setAttribute("aria-pressed", String(button.dataset.setTheme === worn));
    }
    syncMusic();
    syncChaser();
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
    // Click-away closes the floating menu (the nav dropdown stays CSS-only;
    // this one floats over content, so it earns the courtesy).
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
