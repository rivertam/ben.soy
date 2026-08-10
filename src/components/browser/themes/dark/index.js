export const id = "dark";
export const colorScheme = "dark";
export const music = null;

export function tone({ tone, noise }) {
  noise({ d: 0.5, f: 400, q: 0.7, v: 0.6 });
  tone({ f: 90, d: 0.5, w: "sine", v: 0.5, glide: 60 });
}

export function activate() {}
export function deactivate() {}
export function selected() {}
