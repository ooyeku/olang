# Editor support

Your editor can understand olang as well as olang does: errors as you
type, completion, hover help and go-to-definition. That comes from a
language server, a background program editors talk to through a shared
protocol (the Language Server Protocol). It is built into the `olang`
binary as `olang lsp`, so there is nothing extra to install, and the
errors your editor shows come from the very parser and checker that run
your programs. This chapter covers what the server provides and how to
set up VS Code, Zed and other editors.

Part of [the olang book](README.md) · [Command-line tooling](tooling.md)

## What the server provides

| Capability | Source |
|---|---|
| Diagnostics as you type (each with its rule as `code`, as `olang check --format json` writes them) | the real parser (with its line/column info), the semantic analyzer, and the [`olang check`](tooling.md#olang-check) static checker — provable type-annotation violations appear as errors, down to element types (`[1, "a"]` against `List<Int>` flags element 1); warnings sit on the exact declaration span |
| Completions | keywords, the global builtins, the stdlib modules, and `fn`/`type`/`let` names from the open file; inside a string where data is named, its fields: a Frame's columns with their types after `t["`, in `ods.select(t, ["`, `ods.group_by(t, "`, a chain's `\|> ods.sort_by("` and an aggregation's column (the chain's verbs followed: a `group_by` makes its keys and aggregates), a map's keys after `m["` and `map_get(m, "` — from what the program's literals build (`ods.frame([["a", …]])`, `ods.read_csv("a,b\n…")`, `#{ "k": … }`); inside any other string, nothing |
| Cross-file awareness | `use`d modules are resolved and parsed alongside the open file: imported signatures feed the checker's diagnostics, hover shows an imported function's typed signature (marked `// from <file>`), and go-to-definition crosses into the module |
| Hover | the declaration with the checker's type knowledge: annotated signatures in full (`fn dist(a: Float, b: Float) -> Float`), and unannotated `let`s with their inferred types when the checker knows one (`let total: Int`) |
| Go to definition | jumps to the name's declaration — including into the module file that `share`s it |
| Document outline | every `fn`, `meta fn`, `type`, top-level `let`, and `test` block, live while you type |
| References / highlight | every place reading the binding the name under the cursor reads, as olang scopes it: a parameter, a lambda's, a loop's, a match arm's or a block's `let` that shadows an outer name is its own; a module's top-level name is one name wherever it is bound; a name bound nowhere in the file (a builtin, a module) by its spelling; by spelling too while the file does not parse |
| Rename | those places; refuses keywords and standard-library names the file does not bind itself, with a reason |
| Signature help | the signature and active parameter as you type a call — local functions and every registry entry alike |
| Formatting | the `olang fmt` engine — AST-verified, whitespace-only |
| Semantic tokens | names colored by what they bind (a parameter, a function, a module, a type), as olang scopes them |
| Inlay hints | parameter names at call sites, from the file's own declarations and the stdlib's signatures; an argument that already reads as its parameter's name is not hinted |
| Code actions | "Add /// documentation" on an undocumented declaration; "Declare with `let mut`" on an assignment the scoping pass refuses |
| Workspace symbols | every declaration in every `.ol` file under the workspace root whose name contains the query |
| Notebooks | an `.olmd` file is one document: its cells are one module on the file's own lines ([Notebooks](notebooks.md#commands)) |
| Files renamed, moved or deleted | `workspace/didRenameFiles` and `workspace/didDeleteFiles` (registered for olang files and folders): the open documents follow a rename (a folder's too), the old name's problems are cleared, and every open document is checked again — an importer of the name that went or came hears it at once, without an edit |

Positions cross the wire in UTF-16 code units, the protocol's default,
converted at every boundary — a `π` or an emoji earlier in a line never
shifts a hover or a diagnostic. Hover, completion documentation, and
signature help all render from the same registry `:help` prints, so the
editor and the REPL never disagree about what a function is. When a file
is mid-edit and does not parse, declarations fall back to a text scan,
so navigation keeps working while you type.

Positions come from the AST itself: `fn`, `let`, and `type` declarations
carry the source span of the name they bind.

Edits arrive incrementally (a keystroke sends its range and text); the
server parses and checks the whole file again on each one, and keeps
each imported module's parse until that module's source changes.

## VS Code

The extension lives in [`editors/vscode/`](../editors/vscode/) —
syntax highlighting (TextMate grammar covering template strings with
interpolation and escapes, raw strings, `meta fn` and `@` macros,
pipelines, and the module names), bracket/indent behavior, and a thin
client that launches `olang lsp`. The client probes the binary before
starting: a missing or wrong `olang.serverPath` produces an actionable
error with a button to the setting, instead of silently doing nothing.
A [notebook](notebooks.md) (`.olmd`) opens as markdown, its ```` ```olang ````
cells highlighted with the olang grammar; the extension starts the
language server for `.ol` files only.

To install from the repo:

```bash
cd editors/vscode && npm install && npx vsce package && code --install-extension olang-*.vsix
```

If `olang` is not on VS Code's PATH, set `olang.serverPath` in settings.

### If the server does not start

The client resolves the `olang` binary in order: the `olang.serverPath`
setting, then `PATH`, then the well-known install locations (`~/.olang/bin`,
`~/.cargo/bin`, `/usr/local/bin`, `/opt/homebrew/bin`) — a GUI-launched
editor often carries a minimal `PATH` that contains none of them. When
nothing resolves, the extension says so with a button to the setting
rather than failing silently. `olang --version --verbose` and `otc
doctor` show which binary a `PATH` resolves and where it came from, when
an old copy (in `~/.cargo/bin`, say) shadows a newer install.

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
