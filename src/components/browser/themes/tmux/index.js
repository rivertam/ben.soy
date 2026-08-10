// The default adapter describes its palette and attach bell. Session layout,
// navigation, and key handling are core site behavior, not theme lifecycle.

export const id = "tmux";
export const colorScheme = "dark";
export const music = null;

export function tone({ tone, noise }) {
  noise({ d: 0.05, f: 3200, q: 2, v: 0.5 });
  tone({ f: 660, t: 0.07, d: 0.09, w: "square", v: 0.32 });
  tone({ f: 990, t: 0.18, d: 0.14, w: "square", v: 0.28 });
}

export function activate() {}
export function deactivate() {}
export function selected() {}
