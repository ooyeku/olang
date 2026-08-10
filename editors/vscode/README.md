# olang for VS Code

Language support for [olang](https://github.com/ooyeku/olang): syntax
highlighting for `.ol` files plus diagnostics, completions, hover, go to
definition, and formatting via the language server built into the olang
binary (`olang lsp`).

## Requirements

The extension launches `olang lsp` from your PATH. Install olang first —
from a release tarball, `brew install ooyeku/olang/olang`, or a source
build (`make install` in the repository). If the binary lives somewhere
else, point `olang.serverPath` at it in your settings.

## Settings

| Setting | Default | What it does |
|---|---|---|
| `olang.serverPath` | `olang` | Path to the olang binary; the server runs as `olang lsp`. |

## Without the language server

Syntax highlighting and bracket/comment behavior work even when the
binary is missing — the language client simply stays down and the
editor features above it don't activate.
