(function () {
  const highlight = (attempt = 0) => {
    if (window.hljs) {
      window.hljs.highlightAll();
      return;
    }
    if (attempt < 10) {
      setTimeout(() => highlight(attempt + 1), 100);
    }
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => highlight());
  } else {
    highlight();
  }
})();
