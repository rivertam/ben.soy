// The phone-width pane deck on `/`: CSS scroll-snap does the actual swiping;
// this module only keeps the tab bar's aria-current and the address hash
// pointed at the pane in view, so a swiped-to pane is shareable. Desktop
// never constructs the deck scroller (display: contents), which zeroes
// clientWidth and turns every sync into a no-op.

const deck = document.querySelector("[data-pane-deck]");
const tabs = [...document.querySelectorAll("[data-pane-tab]")];

if (deck && tabs.length) {
  const panes = [...deck.querySelectorAll("[data-pane]")];
  let frame = 0;

  const sync = () => {
    frame = 0;
    if (!deck.clientWidth) return;
    const index = Math.round(deck.scrollLeft / deck.clientWidth);
    const pane = panes[Math.max(0, Math.min(panes.length - 1, index))];
    if (!pane?.id) return;
    for (const tab of tabs) {
      if (tab.dataset.paneTab === pane.id) {
        tab.setAttribute("aria-current", "page");
      } else {
        tab.removeAttribute("aria-current");
      }
    }
    // The first pane is the page itself; only the others earn a hash.
    const hash = pane === panes[0] ? "" : `#${pane.id}`;
    if ((location.hash || "") !== hash) {
      history.replaceState(
        history.state,
        "",
        location.pathname + location.search + hash
      );
    }
  };

  deck.addEventListener(
    "scroll",
    () => {
      frame ||= requestAnimationFrame(sync);
    },
    { passive: true }
  );

  // A fragment landing (/#felix) scrolls the deck before this module loads;
  // one initial pass squares the server-rendered aria-current with reality.
  sync();
}
