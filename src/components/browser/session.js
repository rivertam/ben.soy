// Controls that belong specifically to the default tmux session: its status
// bar, prefix, windows, clock, and Vimium notice. Site-wide f/j/k navigation
// and H/L history live in navigation.js and continue under every appearance.

const config = document.querySelector("[data-session-runtime]");
const root = document.documentElement;
const DEFAULT_ID = config.dataset.defaultTheme;
const LAST_WINDOW_KEY = "bens-tmux-last";
const bar = document.querySelector("[data-session-bar]");
const message = document.querySelector("[data-session-message]");

const active = () => !root.dataset.theme || root.dataset.theme === DEFAULT_ID;
const windows = () => [
  ...document.querySelectorAll("[data-session-window]"),
];

let prefixArmed = false;
let prefixTimer = null;
const armPrefix = (on) => {
  prefixArmed = on;
  clearTimeout(prefixTimer);
  prefixTimer = null;
  if (on) {
    if (bar) bar.dataset.prefixArmed = "";
    prefixTimer = setTimeout(() => armPrefix(false), 2000);
  } else if (bar) {
    delete bar.dataset.prefixArmed;
  }
};

const go = (href) => {
  try {
    sessionStorage.setItem(
      LAST_WINDOW_KEY,
      location.pathname + location.search
    );
  } catch {
    // Private mode: last-window simply has nowhere to return to.
  }
  location.href = href;
};

const cycle = (delta) => {
  const items = windows();
  if (!items.length) return;
  const at = Math.max(
    0,
    items.findIndex((item) => item.hasAttribute("aria-current"))
  );
  go(items[(at + delta + items.length) % items.length].getAttribute("href"));
};

const jump = (digit) => {
  const item = windows()[digit];
  if (item) go(item.getAttribute("href"));
};

const lastWindow = () => {
  let last = null;
  try {
    last = sessionStorage.getItem(LAST_WINDOW_KEY);
  } catch {
    // No storage, no bounce.
  }
  if (last && last !== location.pathname + location.search) go(last);
};

let messageTimer = null;
const clearMessage = () => {
  clearTimeout(messageTimer);
  messageTimer = null;
  if (message) {
    message.hidden = true;
    message.textContent = "";
  }
};

const showMessage = (text) => {
  if (!message) return;
  message.textContent = text;
  message.hidden = false;
  clearTimeout(messageTimer);
  messageTimer = setTimeout(clearMessage, 8000);
};

message?.addEventListener("click", clearMessage);

let vimium = null;
let vimiumPromise = null;
const vimiumFeature = () => {
  vimiumPromise ||= import(config.dataset.vimiumModule).then((vimiumModule) => {
    vimium = vimiumModule.createVimiumNotice({
      isActive: active,
      showMessage,
      clearMessage,
    });
    if (active()) vimium.start();
    return vimium;
  });
  return vimiumPromise;
};

let clockTimer = null;
const paintClock = () => {
  const clock = document.querySelector("[data-session-clock]");
  if (clock) clock.textContent = new Date().toTimeString().slice(0, 5);
};

const sync = () => {
  if (active()) {
    paintClock();
    if (!clockTimer) clockTimer = setInterval(paintClock, 20000);
    void vimiumFeature().then((loadedVimium) => {
      if (active()) loadedVimium.start();
    });
  } else {
    clearInterval(clockTimer);
    clockTimer = null;
    vimium?.stop();
    clearMessage();
    armPrefix(false);
  }
};

document.addEventListener("site:appearancechange", sync);
document.addEventListener("site:navigationkey", () => {
  void vimiumFeature().then((loadedVimium) => loadedVimium.markOk());
});

// Capture gives an armed tmux prefix first refusal before the site-wide
// navigation handler sees the following key.
document.addEventListener(
  "keydown",
  (event) => {
    if (!active()) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest("input, textarea, select, [contenteditable='true']")) {
      return;
    }
    if (
      event.ctrlKey &&
      !event.altKey &&
      !event.metaKey &&
      !event.shiftKey &&
      (event.key === "a" || event.key === "b")
    ) {
      event.preventDefault();
      event.stopPropagation();
      armPrefix(!prefixArmed);
      return;
    }
    if (!prefixArmed) return;

    armPrefix(false);
    event.stopPropagation();
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    if (event.key === "n") cycle(1);
    else if (event.key === "p") cycle(-1);
    else if (event.key === "l") lastWindow();
    else if (/^[0-9]$/.test(event.key)) jump(Number(event.key));
    else return;
    event.preventDefault();
  },
  true
);

sync();
