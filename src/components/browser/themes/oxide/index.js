export const id = "oxide";
export const colorScheme = "light";
export const music = null;

export function tone({ tone, noise }) {
  noise({ d: 0.12, f: 2600, q: 4, v: 0.9 });
  tone({ f: 220, d: 0.3, w: "triangle", v: 0.7, glide: 180 });
}

export function activate() {}
export function deactivate() {}
export function selected() {}
