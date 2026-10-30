# Vendored assets

| File | Package | Version | Source | License | Modified |
|---|---|---|---|---|---|
| `mermaid.min.js` | [mermaid](https://www.npmjs.com/package/mermaid) | 11.17.2 | npm tarball (`npm pack mermaid@11`), `dist/mermaid.min.js` (UMD single file) | MIT (`MERMAID-LICENSE.txt`) | no |

Vendored single-file libraries keep the docs site offline-capable: the repository
ships no CDN references.

Size note: `mermaid.min.js` is ~3.5 MB and is **not** loaded on every page -- the
1 KB `../mermaid-init.js` loader injects it only when the page contains
```` ```mermaid ```` fences.

Upgrade procedure: `npm pack mermaid@<version>`, copy `dist/mermaid.min.js` and
`LICENSE` here, bump the row above, rebuild (`pixi run book-build`).
