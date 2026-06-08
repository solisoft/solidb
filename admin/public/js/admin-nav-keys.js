/* Keyboard navigation for the SoliDB admin sidebar.
 *
 * Click anywhere in the sidebar (or focus a link), then Up/Down arrows move
 * focus between the nav links, wrapping at the ends. Tab jumps focus into the
 * main content area (#content) instead of stepping through every link.
 * Delegated on document so bindings survive HTMX #content swaps.
 */
(function () {
  "use strict";
  if (window.__adminNavKeysBound) return;
  window.__adminNavKeysBound = true;

  var LINK_SELECTOR = "#sidebar nav a[href]";

  function navLinks() {
    return Array.prototype.slice.call(document.querySelectorAll(LINK_SELECTOR));
  }

  /* Index of the currently-focused link, or the active page link as a
   * fallback so a fresh click-then-arrow starts from where you are. */
  function currentIndex(links) {
    var focused = links.indexOf(document.activeElement);
    if (focused !== -1) return focused;
    var active = links.findIndex(function (link) {
      return link.classList.contains("text-teal-300");
    });
    return active;
  }

  document.addEventListener("keydown", function (evt) {
    var sidebar = evt.target.closest ? evt.target.closest("#sidebar") : null;
    if (!sidebar) return;

    if (evt.key === "ArrowDown" || evt.key === "ArrowUp") {
      var links = navLinks();
      if (!links.length) return;
      evt.preventDefault();
      var index = currentIndex(links);
      var next;
      if (index === -1) {
        next = evt.key === "ArrowDown" ? 0 : links.length - 1;
      } else {
        next = evt.key === "ArrowDown"
          ? (index + 1) % links.length
          : (index - 1 + links.length) % links.length;
      }
      links[next].focus();
    } else if (evt.key === "Tab" && !evt.shiftKey) {
      var content = document.getElementById("content");
      if (!content) return;
      evt.preventDefault();
      content.focus();
    }
  });

  /* A click on the sidebar background (not a link) leaves nothing focused, so
   * arrows would have no anchor — make the sidebar itself focusable and pull
   * focus to it so the first arrow press has somewhere to start from. */
  document.addEventListener("click", function (evt) {
    var sidebar = evt.target.closest ? evt.target.closest("#sidebar") : null;
    if (!sidebar) return;
    if (evt.target.closest(LINK_SELECTOR)) return;
    sidebar.focus();
  });
})();
