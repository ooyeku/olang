# Editor support

Part of [the olang book](README.md).

olang includes a language server: `olang lsp` speaks the Language Server
Protocol over stdio from the same binary that runs programs. Building the
server into the compiler means there is no separate installation and no
version skew between what the editor reports and what the runtime does, and
every diagnostic comes from the real parser rather than a reimplementation of
it. This chapter covers what the server provides and how to configure the
supported editors.

## What the server provides

| Capability | Source |
|---|---|
| Diagnostics as you type | the real parser (with its line/column info), the semantic analyzer, and the [`olang check`](tooling.md#olang-check) static checker — provable type-annotation violations appear as errors, down to element types (`[1, "a"]` against `List<Int>` flags element 1); warnings sit on the exact declaration span |
| Completions | keywords, the global builtins, the stdlib modules, and `fn`/`type`/`let` names from the open file |
| Cross-file awareness | `use`d modules are resolved and parsed alongside the open file: imported signatures feed the checker's diagnostics, hover shows an imported function's typed signature (marked `// from <file>`), and go-to-definition crosses into the module |
| Hover | the declaration with the checker's type knowledge: annotated signatures in full (`fn dist(a: Float, b: Float) -> Float`), and unannotated `let`s with their inferred types when the checker knows one (`let total: Int`) |
| Go to definition | jumps to the name's declaration — including into the module file that `share`s it |
| Document outline | every `fn`, `meta fn`, `type`, top-level `let`, and `test` block, live while you type |
| References / highlight | every standalone occurrence of the name under the cursor |
| Rename | all occurrences in the file; refuses keywords and standard-library names with a reason |
| Signature help | the signature and active parameter as you type a call — local functions and every registry entry alike |
| Formatting | the `olang fmt` engine — AST-verified, whitespace-only |

Positions cross the wire in UTF-16 code units, the protocol's default,
converted at every boundary — a `π` or an emoji earlier in a line never
shifts a hover or a diagnostic. Hover, completion documentation, and
signature help all render from the same registry `:help` prints, so the
editor and the REPL never disagree about what a function is. When a file
is mid-edit and does not parse, declarations fall back to a text scan,
so navigation keeps working while you type.

Positions come from the AST itself: `fn`, `let`, and `type` declarations
carry the source span of the name they bind.

The server re-parses whole files on every edit. olang files are small
and the parser is fast; correctness stays trivial.

## VS Code

The extension lives in [`editors/vscode/`](../editors/vscode/) —
syntax highlighting (TextMate grammar covering template strings with
interpolation and escapes, raw strings, `meta fn` and `@` macros,
pipelines, and the module names), bracket/indent behavior, and a thin
client that launches `olang lsp`. The client probes the binary before
starting: a missing or wrong `olang.serverPath` produces an actionable
error with a button to the setting, instead of silently doing nothing.

To install from the repo:

```bash
cd editors/vscode && npm install && npx vsce package && code --install-extension olang-0.1.0.vsix
```

If `olang` is not on VS Code's PATH, set `olang.serverPath` in settings.

## Zed

The Zed extension lives in [`editors/zed/`](../editors/zed/), with a
minimal tree-sitter grammar in
[`editors/tree-sitter-olang/`](../editors/tree-sitter-olang/) (token-level,
for highlighting only — the pest grammar in the compiler stays
authoritative). Install it as a dev extension:

1. `zed: install dev extension` from the command palette,
2. pick the `editors/zed/` directory — Zed compiles the extension and
   fetches the grammar,
3. open a `.ol` file: highlighting plus the full server (diagnostics,
   completions, hover, go-to-definition, formatting) via `olang` from
   your PATH.

The grammar reference in `extension.toml` points at this repository
(`editors/tree-sitter-olang`), so the dev-extension flow needs the repo
present locally or the ref pushed.

## Any other LSP editor

Point your editor's LSP client at the command `olang lsp` for the
`olang` language / `.ol` files. For Neovim (0.10+):

```lua
vim.api.nvim_create_autocmd("FileType", {
  pattern = "olang",
  callback = function()
    vim.lsp.start({ name = "olang", cmd = { "olang", "lsp" } })
  end,
})
vim.filetype.add({ extension = { ol = "olang" } })
```

## Protocol coverage

The server is tested at the protocol level — `tests/lsp_test.rs` drives
the real binary over stdio through initialize, didOpen/didChange
diagnostics, completion, formatting, and clean shutdown.
