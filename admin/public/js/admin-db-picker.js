// Header db switcher: filter + keyboard navigation.
// ArrowUp/ArrowDown move the highlight over the *visible* (filtered) options,
// Enter follows the highlighted one. Options are server-rendered anchors
// toggled by Alpine x-show, so visibility is read from style.display.
//
// Plain global (not Alpine.data): the bundled alpine.min.js starts itself in
// a queueMicrotask right after its own script evaluates, before any later
// deferred script runs - an alpine:init listener registered here would fire
// too late. This file is therefore loaded BEFORE alpine.min.js in the layout
// and only defines a global for x-data="dbPicker()" to resolve at init time.
window.dbPicker = function () {
  return {
    open: false,
    q: "",
    hi: 0,

    // Visibility is computed from the same predicate x-show uses (NOT from
    // style.display): x-show applies its DOM update in a later microtask
    // than $nextTick after a filter keystroke, so reading styles here would
    // highlight an option that is about to disappear.
    visibleItems() {
      const query = this.q.toLowerCase();
      return Array.from(this.$refs.dbList.querySelectorAll("a"))
        .filter((a) => query === "" || a.dataset.db.toLowerCase().includes(query));
    },

    // Re-apply the highlight class after any change (filter, move, open).
    // The class is cleared from EVERY anchor (not just visible ones): an
    // option that just got filtered out would otherwise keep a stale
    // highlight.
    refresh() {
      const all = Array.from(this.$refs.dbList.querySelectorAll("a"));
      all.forEach((a) => a.classList.remove("db-option-active"));
      const items = this.visibleItems();
      if (this.hi >= items.length) this.hi = Math.max(items.length - 1, 0);
      if (items[this.hi]) {
        items[this.hi].classList.add("db-option-active");
        // scroll once x-show has actually displayed the element
        this.$nextTick(() => items[this.hi] && items[this.hi].scrollIntoView({ block: "nearest" }));
      }
    },

    move(delta) {
      const count = this.visibleItems().length;
      if (count === 0) return;
      this.hi = (this.hi + delta + count) % count;
      this.refresh();
    },

    choose() {
      const item = this.visibleItems()[this.hi];
      if (item) window.location.href = item.href;
    },

    toggle() {
      this.open = !this.open;
      this.q = "";
      this.hi = 0;
      if (this.open) {
        this.$nextTick(() => this.$refs.dbFilter.focus());
        this.refresh();
      }
    },
  };
};
