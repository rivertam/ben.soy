// Detect the one extension known to capture the site's j/k/f keys.
// The acknowledgement is durable: once those keys reach this page, Vimium is
// either absent or excluded for this origin and there is no reason to nag.

const NOTED_KEY = "bens-tmux-vimium-noted";
const OK_KEY = "bens-tmux-vimium-ok";
const STORE_ID = "dbepggeogbaibhgnhhndojpepiihcmeb";

const store = (key, value) => {
  try {
    if (value === undefined) return localStorage.getItem(key);
    localStorage.setItem(key, value);
  } catch {
    return null;
  }
};

try {
  if (sessionStorage.getItem(NOTED_KEY) === "yes") store(NOTED_KEY, "yes");
} catch {
  // No sessionStorage: nothing to promote from older builds.
}

const installed = async () => {
  if (document.querySelector('[class*="vimium" i], [id*="vimium" i]')) {
    return true;
  }
  if (!("chrome" in window)) return false;
  try {
    await fetch(`chrome-extension://${STORE_ID}/content_scripts/vimium.css`, {
      mode: "no-cors",
    });
    return true;
  } catch {
    return false;
  }
};

export function createVimiumNotice({ isActive, showMessage, clearMessage }) {
  let timer = null;

  const stop = () => {
    if (timer) clearTimeout(timer);
    timer = null;
  };

  const markOk = () => {
    store(OK_KEY, "yes");
    stop();
    clearMessage();
  };

  const note = async () => {
    if (store(NOTED_KEY) === "yes" || store(OK_KEY) === "yes") return;
    if (!(await installed()) || !isActive()) return;
    if (store(OK_KEY) === "yes") return;
    showMessage(
      "vimium detected. First of all, nice. Second, I basically inlined vimium into this site, so try disabling it here. :)"
    );
    store(NOTED_KEY, "yes");
  };

  const start = () => {
    if (
      timer ||
      store(NOTED_KEY) === "yes" ||
      store(OK_KEY) === "yes"
    ) {
      return;
    }
    timer = setTimeout(() => {
      timer = null;
      void note();
    }, 1500);
  };

  return { markOk, start, stop };
}
