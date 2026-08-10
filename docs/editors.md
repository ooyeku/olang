# Editors

olang ships its own language server: `olang lsp` speaks the Language
Server Protocol over stdio, straight from the same binary that runs your
programs — no separate install, always in sync with the compiler.

Part of [the olang book](README.md) ·
[Tour](tour.md) · [Language](language.md) · [Stdlib](stdlib.md)

---

## What the server provides

| capability | source |
|---|---|
| Diagnostics as you type | the real parser (with its line/column info) and the semantic analyzer; warnings sit on the exact declaration span |
| Completions | keywords, the global builtins, the 19 stdlib modules, and `fn`/`type`/`let` names from the open file |
| Hover | the declaration's rendered signature (`fn dist(a, b)`, `type Body`, `let total`) |
| Go to definition | jumps to the name's declaration span in the file |
| Formatting | the `olang fmt` engine — AST-verified, whitespace-only |

Positions come from the AST itself: `fn`, `let`, and `type` declarations
carry the source span of the name they bind.

The server re-parses whole files on every edit. olang files are small
and the parser is fast; correctness stays trivial.

## VS Code

The extension lives in [`editors/vscode/`](../editors/vscode/) —
syntax highlighting (TextMate grammar covering `par for`, pipelines,
template strings, the module names), bracket/indent behavior, and a
thin client that launches `olang lsp`.

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
