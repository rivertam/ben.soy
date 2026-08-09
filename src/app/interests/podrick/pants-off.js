// The /podrick anthem's page chip. Playback, autoplay-on-arrival, volume,
// and the pause preference all live in theme.js's conductor (the <audio>
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
