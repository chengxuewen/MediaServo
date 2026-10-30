---
name: lsp-code-analysis
description: "Semantic pre-change impact analysis for MediaServo: find definitions, implementors, references, and call paths across Rust crates, C ABI/FFI, and the Admin Dashboard before editing a symbol. Uses serena LSP first, codegraph/ripgrep as fallback."
---

# LSP Code Analysis — Pre-Change Impact

> Ask what breaks before you change the symbol. The file tree is not the call graph.

## Trigger Conditions

- Before changing a public trait, enum, or function used across crates.
- Before touching the C ABI/FFI surface or a wire contract.
- Before renaming a symbol shared by server, SFU, and UI.
- Before removing a plugin/media extension point.
- 用户说“改这个会影响谁 / 找实现 / 找引用 / 调用链 / 影响面” <!-- c49:trigger -->

## Do Not Use

- The layer is still unknown: use `repo-triage` first.
- The contract is being designed, not changed: use `api-interface-design`.
- The task is docs-only: text search is enough.

## Tool Ladder

### 1. serena (preferred, LSP-backed)

The serena MCP server is the semantic source. Active tools:

| Question | Tool |
|---|---|
| Where is the symbol defined? | `find_symbol`, `find_declaration` |
| Who implements this trait? | `find_implementations` |
| Who references it? | `find_referencing_symbols` |
| What is in this file? | `get_symbols_overview` |
| Find a pattern across the project | `search_for_pattern` |
| Rename with LSP safety | `rename_symbol` |

Use the full symbol path when the name is ambiguous. Read the returned file/range,
not only the symbol name.

### 2. codegraph (fallback + call trail)

```bash
CG=scripts/agents/node_modules/.mcp/local-codegraph/node_modules/.bin/codegraph
"$CG" query "Plugin"
"$CG" node "Plugin"
"$CG" explore "plugin registration lifecycle"
```

- `query` finds symbols by name.
- `node` shows source + caller/callee trail.
- `explore` returns relevant symbols and call paths in one shot.
- The index is built by `./msrtc.sh mcp index` (parent root) or `pixi run mcp index`.

### 3. ripgrep (last resort)

```bash
.pixi/envs/default/bin/rg -n 'pub trait Plugin|impl Plugin' crates
```

Ripgrep hits comments and same-name items. Treat the result as a **superset** and
prune it manually; state that the LSP was unavailable.

## Workflow

1. **Name the symbol and the intended change.** One sentence: "change `Plugin::start` signature" or "remove `SignalingMessage::Legacy`".
2. **Find the definition.** Confirm the owning crate and the exact item.
3. **Find implementors and references.** Trait first, then callers.
4. **Read the call paths.** Identify entry points (server, FFI, UI) and teardown paths.
5. **Map the blast radius.**

   | Layer | Check |
   |---|---|
   | Rust crates | implementors, callers, feature flags |
   | C ABI / FFI | exported symbols, ownership, C++ callers |
   | Wire | serde shape, versioning, client compatibility |
   | Admin Dashboard | WS/REST consumer, state/store, playback |
   | Tests | contract tests that must change |
   | Gates | `check`, `test`, `gate-abi`, `gate-openapi`, `book-check` |

6. **List what must change and what must not.** A symbol with no external caller is internal; a trait implementor outside the crate is a contract.
7. **Stop if the blast radius exceeds the slice.** Split by contract; do not edit 20 files in one pass.

## Worked Example (MediaServo)

```bash
CG=scripts/agents/node_modules/.mcp/local-codegraph/node_modules/.bin/codegraph
"$CG" query "Plugin"
```

Observed result: the trait `Plugin` at
`crates/mediaservo-media/src/plugin.rs:25`, plus `PluginKind`, `PluginManager`,
and `PluginCapability`. That is the minimum map; the next step is
`find_implementations` in serena and `node Plugin` for the call trail.

## Output Format

```text
Symbol: <path::symbol>
Change: <signature/behavior/removal>
Definition: <file:line>
Implementors: <list>
References: <count + hotspots>
Call paths: <entry -> ... -> teardown>
Affected layers: <crates | FFI | wire | UI | tests>
Gates: <names>
Decision: safe to change | split | stop and redesign
```

## Degradation Rules

- No serena/LSP: use codegraph; label it "index-based, may miss macro-generated items".
- No codegraph index: use ripgrep; label it "text-only superset".
- Never claim "no references" from one tool; cross-check at least two sources for a
  public symbol.

## Boundaries

- `repo-triage`: chooses the layer; this skill maps the symbol's impact.
- `api-interface-design`: decides the contract; this skill measures the blast radius.
- `code-simplification`: changes the code; this skill protects the callers.
- `test-driven-development`: pins the behavior; this skill lists the tests to update.
