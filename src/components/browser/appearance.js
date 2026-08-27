// Host for the site's homogeneous theme packages. Rust renders one
// registration per package with its fingerprinted module and optional asset
// URLs; this module knows nothing about individual theme IDs.
//
// Every package exports:
//   id, colorScheme, music,
//   tone(sound), activate(context), deactivate(context), selected(context)
// CSS stays eagerly compiled for correct first paint. Package JavaScript is
// imported only for the worn (or pointer/focus-preloaded) theme.

const config = document.querySelector("[data-appearance-runtime]");
const root = document.documentElement;
const KEY = config.dataset.themeKey;
const DAY_OVERRIDE_KEY = config.dataset.themeDayOverrideKey;
const MUSIC_KEY = "bens-theme-music";
const VOLUME_KEY = "bens-theme-volume";
const DEFAULT_ID = config.dataset.defaultTheme;
const WEEKLY_ID = config.dataset.weeklyTheme;
const WEEKLY_DAY = Number(config.dataset.weeklyThemeDay);
const themeButtons = [
  ...document.querySelectorAll("[data-set-theme][data-theme-module]"),
];
const registrations = new Map(
  themeButtons.map((button) => {
    const id = button.dataset.setTheme;
    return [
      id,
      {
        id,
        module: button.dataset.themeModule,
        assets: Object.freeze({
          music: button.dataset.themeMusic || null,
          image: button.dataset.themeImage || null,
        }),
        theme: null,
        promise: null,
        context: null,
      },
    ];
  })
);

const calendarStamp = (date = new Date()) =>
  `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
const isWeeklyDay = (date = new Date()) =>
  registrations.has(WEEKLY_ID) &&
  Number.isInteger(WEEKLY_DAY) &&
  date.getDay() === WEEKLY_DAY;
const storedPreference = () => {
  try {
    const stored = localStorage.getItem(KEY);
    return registrations.has(stored) ? stored : DEFAULT_ID;
  } catch {
    return DEFAULT_ID;
  }
};
const automaticTheme = (date = new Date()) => {
  if (!isWeeklyDay(date)) return null;
  try {
    if (localStorage.getItem(DAY_OVERRIDE_KEY) === calendarStamp(date)) {
      return null;
    }
  } catch {
    // Without storage, the weekly tradition still applies.
  }
  return WEEKLY_ID;
};
const effectiveTheme = (date = new Date()) =>
  automaticTheme(date) || storedPreference();
const rememberSelection = (id, date = new Date()) => {
  try {
    // A deliberate Thursday choice wins for the rest of the local date and
    // survives navigation, while tomorrow still starts from the preference.
    if (isWeeklyDay(date)) {
      localStorage.setItem(DAY_OVERRIDE_KEY, calendarStamp(date));
    } else {
      localStorage.removeItem(DAY_OVERRIDE_KEY);
    }
    if (id === DEFAULT_ID) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, id);
  } catch {
    // Private-mode storage failure: the choice still applies to this page.
  }
};
const expireDayOverride = (date = new Date()) => {
  try {
    const override = localStorage.getItem(DAY_OVERRIDE_KEY);
    if (override && override !== calendarStamp(date)) {
      localStorage.removeItem(DAY_OVERRIDE_KEY);
    }
  } catch {
    // No storage: there is nothing durable to expire.
  }
};

const wornId = () => root.dataset.theme || DEFAULT_ID;
const reducedMotion = () =>
  matchMedia("(prefers-reduced-motion: reduce)").matches;
const band = () => document.querySelector("[data-page-band]");

let vol = 0.5;
try {
  const stored = parseFloat(localStorage.getItem(VOLUME_KEY));
  if (Number.isFinite(stored)) vol = Math.min(1, Math.max(0, stored));
} catch {
  // No storage: the designed level stands.
}

let ctx = null;
let stingBus = null;
let musicBus = null;
let tuneEl = null;

const applyVolume = () => {
  if (stingBus) stingBus.gain.value = 0.11 * vol;
  if (musicBus) musicBus.gain.value = 0.09 * vol;
  if (tuneEl) tuneEl.volume = Math.min(1, 0.7 * vol);
  const pageBand = band();
  if (pageBand) pageBand.volume = Math.min(1, 1.2 * vol);
};

/* ── audio engine ───────────────────────────────────────────────────── */

const audio = () => {
  if (!ctx) {
    const AudioContext = window.AudioContext || window.webkitAudioContext;
    if (!AudioContext) return null;
    ctx = new AudioContext();
    stingBus = ctx.createGain();
    stingBus.connect(ctx.destination);
    musicBus = ctx.createGain();
    musicBus.connect(ctx.destination);
    applyVolume();
  }
  if (ctx.state === "suspended") void ctx.resume();
  return ctx;
};

const playTone = (
  bus,
  { f, t = 0, d = 0.15, w = "sine", v = 1, glide }
) => {
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

const playNoise = (
  bus,
  { t = 0, d = 0.08, f = 2000, q = 1, v = 1 }
) => {
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

// Packages get a deliberately tiny sound interface, pre-bound to the sting
// bus. Returning false lets a lifecycle effect defer sound until the browser
// has granted an AudioContext.
const themeSound = Object.freeze({
  tone(options) {
    if (!ctx || !stingBus) return false;
    playTone(stingBus, options);
    return true;
  },
  noise(options) {
    if (!ctx || !stingBus) return false;
    playNoise(stingBus, options);
    return true;
  },
});

/* ── package interface ───────────────────────────────────────────────── */

const PACKAGE_FUNCTIONS = ["tone", "activate", "deactivate", "selected"];

const validatePackage = (registration, theme) => {
  if (theme.id !== registration.id) {
    throw new TypeError(
      `theme package "${registration.id}" exports id "${theme.id}"`
    );
  }
  if (theme.colorScheme !== "light" && theme.colorScheme !== "dark") {
    throw new TypeError(
      `theme package "${registration.id}" needs a light/dark colorScheme`
    );
  }
  if (!("music" in theme)) {
    throw new TypeError(
      `theme package "${registration.id}" must export music (or null)`
    );
  }
  for (const name of PACKAGE_FUNCTIONS) {
    if (typeof theme[name] !== "function") {
      throw new TypeError(
        `theme package "${registration.id}" must export ${name}()`
      );
    }
  }
  if (theme.music !== null) {
    if (theme.music?.kind === "audio") {
      if (
        typeof theme.music.asset !== "string" ||
        !registration.assets[theme.music.asset]
      ) {
        throw new TypeError(
          `theme package "${registration.id}" has no "${theme.music.asset}" asset`
        );
      }
    } else if (
      theme.music?.kind !== "sequence" ||
      !Number.isFinite(theme.music.bpm) ||
      !Array.isArray(theme.music.bass) ||
      !Array.isArray(theme.music.lead) ||
      !Array.isArray(theme.music.perc)
    ) {
      throw new TypeError(
        `theme package "${registration.id}" has invalid music`
      );
    }
  }
  return theme;
};

const loadTheme = (id) => {
  const registration = registrations.get(id);
  if (!registration) return Promise.resolve(null);
  registration.promise ||= import(registration.module)
    .then((theme) => {
      registration.theme = validatePackage(registration, theme);
      return registration.theme;
    })
    .catch((error) => {
      console.error(`Could not load theme package "${id}"`, error);
      return null;
    });
  return registration.promise;
};

const themeContext = (registration) => {
  registration.context ||= Object.freeze({
    assets: registration.assets,
    reducedMotion,
    sound: themeSound,
  });
  return registration.context;
};

const preloadTheme = (event) => {
  const target = event.target instanceof Element ? event.target : null;
  const button = target?.closest("[data-set-theme][data-theme-module]");
  if (button) void loadTheme(button.dataset.setTheme);
};

document.addEventListener("pointerover", preloadTheme, { passive: true });
document.addEventListener("focusin", preloadTheme);

/* ── music ──────────────────────────────────────────────────────────── */

const musicFor = (id) => registrations.get(id)?.theme?.music || null;
const audioSourceFor = (id) => {
  const registration = registrations.get(id);
  const music = registration?.theme?.music;
  return music?.kind === "audio"
    ? registration.assets[music.asset] || null
    : null;
};
const midi = (note) => 440 * Math.pow(2, (note - 69) / 12);

let musicOn = false;
let step = 0;
let nextAt = 0;
let timer = null;
let mp3Timer = null;
let playing = null;
let startDelay = 0.05;

const scheduleAhead = () => {
  const sequence = musicFor(playing);
  if (sequence?.kind !== "sequence") return;
  const steps = Math.max(
    sequence.bass.length,
    sequence.lead.length,
    sequence.perc.length
  );
  const stepDuration = 60 / sequence.bpm / 4;
  while (nextAt < ctx.currentTime + 0.25) {
    const t = nextAt - ctx.currentTime;
    const i = step % steps;
    if (sequence.bass[i] != null) {
      playTone(musicBus, {
        f: midi(sequence.bass[i]),
        t,
        d: stepDuration * 1.8,
        w: sequence.wave,
        v: 0.5,
      });
    }
    if (sequence.lead[i] != null) {
      playTone(musicBus, {
        f: midi(sequence.lead[i]),
        t,
        d: stepDuration * 2.6,
        w: sequence.wave,
        v: 0.3,
      });
    }
    if (sequence.perc[i]) {
      playNoise(musicBus, { t, d: 0.025, f: 4000, q: 1.5, v: 0.4 });
    }
    nextAt += stepDuration;
    step += 1;
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

const startMusic = () => {
  if (!audio()) return;
  const id = wornId();
  const music = musicFor(id);
  if (!music) {
    stopMusic();
    return;
  }
  if (playing === id) return;
  stopMusic();
  playing = id;
  const delay = startDelay;
  startDelay = 0.05;
  if (music.kind === "audio") {
    const src = audioSourceFor(id);
    if (!src) {
      playing = null;
      return;
    }
    if (!tuneEl) {
      tuneEl = new Audio();
      tuneEl.loop = true;
    }
    if (tuneEl.getAttribute("src") !== src) tuneEl.src = src;
    applyVolume();
    mp3Timer = setTimeout(() => {
      tuneEl.play().catch(() => {
        // Autoplay said no; the next gesture will retry.
        playing = null;
        syncTransport();
      });
    }, delay * 1000);
  } else {
    step = 0;
    nextAt = ctx.currentTime + delay;
    scheduleAhead();
    timer = setInterval(scheduleAhead, 100);
  }
};

const syncTransport = () => {
  const wrap = document.querySelector(".band-wrap");
  const pill = document.querySelector(".band-pill");
  if (!wrap || !pill) return;
  const pageBand = band();
  const source = pageBand || musicFor(wornId());
  wrap.hidden = !source;
  const sounding = pageBand ? !pageBand.paused && !pageBand.ended : !!playing;
  pill.textContent = sounding ? "\u23F8" : "\u25B6";
};

const syncMusic = () => {
  const pageBand = band();
  if (pageBand) {
    stopMusic();
    if (musicOn && !document.hidden) {
      applyVolume();
      pageBand.play().catch(() => {
        // Autoplay veto: the first-gesture listener retries.
      });
    } else {
      pageBand.pause();
    }
  } else {
    const carries = musicFor(wornId());
    if (ctx && musicOn && carries && !document.hidden) startMusic();
    else stopMusic();
  }
  syncTransport();
};

const setMusic = (on, { fromGesture = true } = {}) => {
  musicOn = on;
  document.querySelectorAll("[data-music-toggle]").forEach((button) => {
    button.setAttribute("aria-pressed", String(on));
  });
  if (fromGesture) {
    try {
      localStorage.setItem(MUSIC_KEY, on ? "on" : "off");
    } catch {
      // Private mode: the tune still plays for this page.
    }
    audio();
  }
  syncMusic();
};

document.addEventListener("visibilitychange", () => {
  syncMusic();
  if (!document.hidden) syncCalendarAppearance();
});

/* ── lifecycle and switching ────────────────────────────────────────── */

let activeTheme = null;
let activeContext = null;
let activationEpoch = 0;

const activateTheme = (
  id,
  { selected = false, audioReady = false } = {}
) => {
  const epoch = ++activationEpoch;
  if (activeTheme && activeTheme.id !== id) {
    activeTheme.deactivate(activeContext);
    activeTheme = null;
    activeContext = null;
  }

  void loadTheme(id).then((theme) => {
    if (!theme || epoch !== activationEpoch || wornId() !== id) return;
    const registration = registrations.get(id);
    const context = themeContext(registration);
    activeTheme = theme;
    activeContext = context;
    theme.activate(context);
    if (selected) {
      if (audioReady && audio()) theme.tone(themeSound);
      if (musicOn && theme.music && playing !== id) startDelay = 1;
      theme.selected(context);
    }
    syncMusic();
  });
};

const apply = (requested, options) => {
  const id = registrations.has(requested) ? requested : DEFAULT_ID;
  if (id === DEFAULT_ID) delete root.dataset.theme;
  else root.dataset.theme = id;

  for (const button of themeButtons) {
    button.setAttribute(
      "aria-pressed",
      String(button.dataset.setTheme === id)
    );
  }
  // Stop the previous package's music immediately. Loading the next package
  // will call this again once its optional music descriptor is available.
  syncMusic();
  activateTheme(id, options);
  document.dispatchEvent(
    new CustomEvent("site:appearancechange", { detail: { id } })
  );
};

const millisecondsUntilTomorrow = (now = new Date()) => {
  const tomorrow = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() + 1
  );
  return Math.max(1000, tomorrow.getTime() - now.getTime() + 100);
};

let calendarDay = calendarStamp();
let calendarTimer = null;
const scheduleCalendarSync = () => {
  if (calendarTimer) clearTimeout(calendarTimer);
  calendarTimer = setTimeout(
    syncCalendarAppearance,
    millisecondsUntilTomorrow()
  );
};
const syncCalendarAppearance = () => {
  const now = new Date();
  const today = calendarStamp(now);
  if (today !== calendarDay) {
    calendarDay = today;
    expireDayOverride(now);
    const effective = effectiveTheme(now);
    if (effective !== wornId()) apply(effective);
  }
  scheduleCalendarSync();
};

document.addEventListener("click", (event) => {
  const target = event.target instanceof Element ? event.target : null;
  if (!target) return;

  const musicButton = target.closest("[data-music-toggle]");
  if (musicButton) {
    const pageBand = band();
    const sounding = pageBand
      ? !pageBand.paused && !pageBand.ended
      : !!playing;
    audio();
    setMusic(!(musicOn && sounding));
    return;
  }

  const button = target.closest("[data-set-theme][data-theme-module]");
  if (button) {
    const id = button.dataset.setTheme;
    const audioReady = !!audio();
    apply(id, { selected: true, audioReady });
    rememberSelection(id);
    button.closest("details")?.removeAttribute("open");
    return;
  }

  const open = document.querySelector(".theme-dd[open]");
  if (open && !open.contains(target)) open.removeAttribute("open");
});

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

addEventListener("storage", (event) => {
  if (event.key === KEY || event.key === DAY_OVERRIDE_KEY) {
    apply(effectiveTheme());
  }
  if (event.key === MUSIC_KEY) {
    setMusic(event.newValue !== "off", { fromGesture: false });
  }
});

const bootSlider = document.querySelector(".band-volume");
if (bootSlider) bootSlider.value = String(Math.round(vol * 100));

{
  const pageBand = band();
  if (pageBand) {
    for (const kind of ["play", "pause", "ended"]) {
      pageBand.addEventListener(kind, syncTransport);
    }
  }
}

apply(effectiveTheme());
scheduleCalendarSync();
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
