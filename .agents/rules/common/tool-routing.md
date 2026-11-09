# Tool Routing -- exploration tool division of labor

> **Target audience**: AI agents searching/exploring this repository.
> The split follows the community consensus (agentic search vs index-backed
> search): use the fast path first, escalate only when needed.
> **This file defines boundaries, not priorities** -- the host's own search
> commands (e.g. native SearchCodebase) are not overridden by it.
> Note: the semantic-row tool choice below is the initial draft; the final
> wording is owned by the index-attribution round (B7-1). If the index proves
> unreliable there, this row is rewritten with LSP in the primary seat.

## Division matrix

| Query type | Tool | Why |
|----------|--------|------|
| Exact text / known symbol / exact citation | Native Grep / LSP (findReferences / goToDefinition) | Fastest, explainable results (ripgrep ~20ms class) |
| **Semantic exploration: how does X work / call paths / impact surface / unfamiliar-domain survey / pre-change context packing** | `codegraph_explore` (MCP `local-codegraph`) | One call returns verbatim source of related symbols (grouped by file, counts as read) plus inter-symbol call paths; index covers cpp/c/rust, saving rounds and tokens |
| Fuzzy zone solvable by either | Pick **one path** and finish it | Running two retrieval paths for the same question is forbidden (double tokens, zero gain) |

## Usage constraints

- Source code returned by `codegraph_explore` **counts as Read**; do not re-Read
  the same file (server contract).
- When the index is missing or the path is not indexed, the server returns
  guidance to fall back to built-in tools -- follow it, do not retry the MCP.
- When the host has not injected `local-codegraph`, this matrix degrades to the
  pure native-tool path -- that is a working configuration, not a failure.

## Lock-in convention

This file and the root `.mcp.json` `local-codegraph` entry lock each other
(the machine check lands with `gate-mcp`, B5-4):
deleting the server must delete/rename this file in the same change; keeping
the server requires this file to exist.
