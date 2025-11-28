(function () {
  const highlight = () => {
    if (!window.hljs) { return; }
    document.querySelectorAll('pre code[class^="language-"]').forEach((block) => {
      window.hljs.highlightElement(block);
    });
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", highlight);
  } else {
    highlight();
  }
})();
