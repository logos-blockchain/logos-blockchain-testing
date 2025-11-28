// Lightweight client-side Mermaid rendering for mdBook.
(function () {
  const CDN = "https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js";

  function loadMermaid(cb) {
    if (window.mermaid) {
      cb();
      return;
    }
    const script = document.createElement("script");
    script.src = CDN;
    script.onload = cb;
    script.onerror = () => console.warn("Failed to load mermaid from CDN:", CDN);
    document.head.appendChild(script);
  }

  function renderMermaidBlocks() {
    const codeBlocks = Array.from(
      document.querySelectorAll("pre code.language-mermaid")
    );
    if (codeBlocks.length === 0) {
      return;
    }

    codeBlocks.forEach((codeBlock, idx) => {
      const pre = codeBlock.parentElement;
      const container = document.createElement("div");
      container.className = "mermaid";
      container.textContent = codeBlock.textContent;
      container.id = `mermaid-diagram-${idx}`;
      pre.replaceWith(container);
    });

    if (window.mermaid) {
      window.mermaid.initialize({ startOnLoad: false });
      window.mermaid.run();
    }
  }

  document.addEventListener("DOMContentLoaded", () => {
    loadMermaid(renderMermaidBlocks);
  });
})();
