// The /podrick anthem's page chip. Playback, autoplay-on-arrival, volume,
// and the pause preference all live in appearance.js's conductor (the <audio>
// carries data-page-band and the chip carries data-music-toggle); this
// file only keeps the chip's label honest as the element's state changes.
const audio = document.getElementById("anthem");
const toggle = document.getElementById("anthem-toggle");
if (audio && toggle) {
  const paint = () => {
    const playing = !audio.paused && !audio.ended;
    toggle.textContent = playing
      ? "⏸ pants off o'clock"
      : "▶ pants off o'clock";
  };
  for (const kind of ["play", "pause", "ended"]) {
    audio.addEventListener(kind, paint);
  }
  paint();
}

// Year changes are Topcoat shard updates rather than document navigations, so
// the anthem and the rest of the page keep their DOM state. The anchors stay
// usable as ordinary links without JavaScript and for modified clicks.
const yearInput = document.querySelector("[data-pants-year-input]");
if (yearInput) {
  const selectLocation = (url) => {
    const selected = url.searchParams.get("year") || yearInput.dataset.currentYear;
    if (!selected || yearInput.value === selected) return;
    yearInput.value = selected;
    yearInput.dispatchEvent(new Event("input", { bubbles: true }));
  };

  document.addEventListener("click", (event) => {
    const link = event.target.closest?.("[data-pants-year-link]");
    if (
      !link ||
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }

    const url = new URL(link.href, window.location.href);
    if (url.origin !== window.location.origin) return;
    event.preventDefault();
    window.history.pushState(null, "", `${url.pathname}${url.search}${url.hash}`);
    selectLocation(url);
  });

  window.addEventListener("popstate", () => {
    selectLocation(new URL(window.location.href));
  });
}
