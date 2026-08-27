// Plaid Thursday's adapter. The weave is CSS; selection sounds like a quick
// brush across shirt fabric.

export const id = "plaid";
export const colorScheme = "dark";
export const music = null;

export function tone({ noise, tone }) {
  noise({ d: 0.18, f: 1450, q: 0.65, v: 0.42 });
  tone({ f: 196, t: 0.04, d: 0.16, w: "triangle", v: 0.2 });
}

export function activate() {}
export function deactivate() {}
export function selected() {}
