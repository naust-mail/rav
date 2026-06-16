// Minimal inert polyfill for browsers that don't support the native `inert` attribute
// (all modern evergreen browsers do, this only matters for older clients).
// Replaces the wicg-inert npm package after a supply-chain scanner flagged it.
if (typeof window !== "undefined" && !("inert" in HTMLElement.prototype)) {
  const blockEvent = (event: Event) => {
    if ((event.target as Element | null)?.closest("[inert]")) {
      event.stopPropagation();
      event.preventDefault();
    }
  };

  for (const type of ["click", "mousedown", "mouseup", "keydown", "keyup", "focus"]) {
    document.addEventListener(type, blockEvent, true);
  }

  Object.defineProperty(HTMLElement.prototype, "inert", {
    enumerable: true,
    get(this: HTMLElement) {
      return this.hasAttribute("inert");
    },
    set(this: HTMLElement, value: boolean) {
      if (value) {
        this.setAttribute("inert", "");
        this.setAttribute("aria-hidden", "true");
      } else {
        this.removeAttribute("inert");
        this.removeAttribute("aria-hidden");
      }
    },
  });
}
