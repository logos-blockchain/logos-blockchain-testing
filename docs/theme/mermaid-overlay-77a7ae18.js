(function () {
  const openOverlay = (svg) => {
    const overlay = document.createElement("div");
    overlay.className = "mermaid-overlay";

    const content = document.createElement("div");
    content.className = "mermaid-overlay__content";

    const clone = svg.cloneNode(true);
    clone.removeAttribute("width");
    clone.removeAttribute("height");
    clone.style.width = "95vw";
    clone.style.maxWidth = "1400px";
    clone.style.height = "auto";
    clone.style.display = "block";
    clone.style.margin = "0 auto";

    content.appendChild(clone);
    overlay.appendChild(content);
    document.body.appendChild(overlay);

    const close = () => overlay.remove();
    overlay.addEventListener("click", close);
    document.addEventListener(
      "keydown",
      (e) => {
        if (e.key === "Escape") {
          close();
        }
      },
      { once: true }
    );
  };

  const bind = () => {
    document.querySelectorAll(".mermaid svg").forEach((svg) => {
      if (svg.dataset.overlayBound === "true") {
        return;
      }
      svg.style.cursor = "zoom-in";
      svg.addEventListener("click", () => openOverlay(svg));
      svg.dataset.overlayBound = "true";
    });
  };

  const init = () => {
    bind();
    // Mermaid renders asynchronously; bind again after a short delay.
    setTimeout(bind, 500);
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
