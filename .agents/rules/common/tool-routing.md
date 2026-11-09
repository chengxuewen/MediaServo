# Tool Routing -- exploration tool division of labor

> **Target audience**: AI agents searching/exploring this repository.
> **This file defines boundaries, not priorities** -- the host's own search
> commands (e.g. native SearchCodebase) are not overridden by it.
> Attribution basis (B7-1 index-attribution round, 2026-11-09): 81% of the
> 38.7k unresolved index refs are out-of-repo symbols (std / external crates)
> plus call-chain tail fragments -- classes the index cannot resolve *by
> construction*. The field pair-test (B7-5) confirmed it: LSP answers exact
> (2/2, zero noise), codegraph answers fast but with common-name noise.
> Hence the seats below.

## Division matrix

| Query type | Tool | Why |
|----------|--------|------|
| Exact text / known symbol / exact citation | Native Grep / LSP (findReferences / goToDefinition) | Fastest, explainable results (ripgrep ~20ms class) |
| **Call paths / impact surface / pre-edit blast radius / macro-generated items / external-crate or std symbols / trait dispatch** | serena (LSP) -- via `local-serena` MCP or the host's LSP | Type-system-backed: resolves the exact classes the index cannot (B7-1); pair-test 2/2 exact with zero noise (B7-5) |
| **In-repo structure overview / unfamiliar-area survey / quick blast radius across many files** | `codegraph_explore` (MCP `local-codegraph`) | One call returns related symbols' verbatim source (counts as read) + call paths in <1s; covers cpp/c/rust; common-name noise possible -- confirm with LSP before editing |
| **Browser-facing verification / runtime page behavior / console & network observation** | playwright (`local-playwright` MCP) | The only face that sees the running page; procedure owned by the `browser-testing` skill |
| Fuzzy zone solvable by either | Pick **one path** and finish it | Running two retrieval paths for the same question is forbidden (double tokens, zero gain) |

## Usage constraints

- Source code returned by any MCP tool **counts as Read**; do not re-Read
  the same file (server contract).
- When the index is missing or the path is not indexed, the server returns
  guidance to fall back to built-in tools -- follow it, do not retry the MCP.
- LSP not injected in the host? The serena MCP server provides the same
  semantics; if neither is present, `codegraph_explore` is the fallback for
  structure questions -- mind the common-name noise, verify with Grep.
- When a server is not injected at all, this matrix degrades to the pure
  native-tool path -- that is a working configuration, not a failure.
- serena's project `memories` carry pointer-type project knowledge for
  cross-session recall (B7-3); they point at the true sources, never
  duplicate them.

## Lock-in convention

Each of the three code-facing servers (`local-codegraph`, `local-serena`,
`local-playwright`) and the rows above lock each other; the machine check
lives in `gate_mcp.py` (B5-4): a host file naming a server requires this
file to exist and carry that server's matrix row, and removing a server
must remove its row in the same change.
