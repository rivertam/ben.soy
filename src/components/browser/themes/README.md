# Theme packages

Every directory here is one adapter at the appearance runtime's package seam:

```text
<id>/
  index.js   browser behavior and metadata
  theme.css  palette and presentation
```

`index.js` has the same named exports in every package:

- `id` — exactly matches the Rust registry id
- `colorScheme` — `"light"` or `"dark"`, matching `theme.css`
- `music` — `null`, an `audio` descriptor naming a Rust-provided asset, or a
  synthesized `sequence` descriptor
- `tone(sound)` — the short acknowledgement played on explicit selection
- `activate(context)` / `deactivate(context)` — lifecycle hooks
- `selected(context)` — effects that happen only on an explicit selection

The context contains named `assets`, a live `reducedMotion()` query, and the
small `sound` interface. `src/content/themes.rs` owns labels and fingerprinted
asset registration; `styles/themes.css` eagerly imports package CSS for a
flash-free first paint; `browser/appearance.js` validates and lazily imports
package JavaScript.

Tmux uses this interface for its palette and attach bell, but the tmux session
is the site's default chrome. Its rendering and controls therefore stay in
Rust, `styles/session.css`, and `browser/session*.js` rather than in the theme
lifecycle.
