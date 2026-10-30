// Lazy mermaid loader for the docs site (S3).
//
// Loaded on every page via [output.html] additional-js, but it does nothing
// unless the page actually contains ```mermaid fences -- the 3.5 MB library is
// fetched only where a diagram exists. The vendor path is resolved relative to
// this script, so it works at any page depth.

(() => {
  const fences = Array.from(document.querySelectorAll("pre > code.language-mermaid"));
  if (!fences.length) return;

  const self = document.currentScript ||
    Array.from(document.scripts).find((s) => s.src.includes("mermaid-init"));
  const base = self ? new URL(".", self.src) : new URL("/web/", location.href);

  // Hide the raw fences while the renderer loads (no flash of code).
  const style = document.createElement("style");
  style.textContent = "pre:has(> code.language-mermaid){visibility:hidden}";
  document.head.appendChild(style);

  const script = document.createElement("script");
  script.src = new URL("vendor/mermaid.min.js", base).href;
  script.onload = () => {
    fences.forEach((code) => {
      const box = document.createElement("div");
      box.className = "mermaid";
      box.textContent = code.textContent;
      code.closest("pre").replaceWith(box);
    });
    style.remove();
    const mermaid = window.mermaid;
    if (!mermaid) { console.error("mermaid global missing"); return; }
    mermaid.initialize({ startOnLoad: false, theme: "neutral", securityLevel: "loose" });
    mermaid.run({ nodes: document.querySelectorAll(".mermaid") }).catch((err) => console.error(err));
  };
  script.onerror = () => { style.remove(); console.error("mermaid failed to load"); };
  document.head.appendChild(script);
})();
