(function () {
  const load = () => {
    // eslint-disable-next-line no-undef
    mermaid.initialize({
      startOnLoad: true,
      theme: "default",
      securityLevel: "loose",
    });
  };

  if (window.mermaid) {
    load();
    return;
  }

  const script = document.createElement("script");
  script.src = "https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js";
  script.onload = load;
  document.head.appendChild(script);
})();
