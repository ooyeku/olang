# Installation

This chapter describes how to install the olang compiler, run programs, use
the interactive REPL, and run the language in the browser without installing
anything.

Part of [the olang book](README.md).

## Requirements

olang is built with Cargo, the Rust package manager. Installing from source
requires a recent stable Rust toolchain, available from
[rustup.rs](https://rustup.rs). No other dependencies are required; the SQLite
library used by the `db` module is bundled and built from source.

## Installing from source

Clone the repository and install the `olang` binary with Cargo:

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
cargo install --path .
```

This builds an optimized binary and places it on the Cargo binary path
(typically `~/.cargo/bin`). Confirm the installation:

```bash
olang --version
```

To build without installing — for development on the compiler itself — use
`cargo build --release`; the binary is then at `target/release/olang`.

## Running programs

Pass a file to run it. Any arguments after the file name are passed to the
program and are readable through `os.args()`:

```bash
olang script.ol
olang script.ol --verbose input.csv
```

A file may also be made executable directly with a shebang line, since olang
ignores a leading `#!` line:

```
#!/usr/bin/env olang
println("hello")
```

The `olang <file>` form is the file-first invocation. `olang run <file>` is
an equivalent explicit form. A word that is neither a known command nor a
flag is treated as a file path, so both `olang report.ol` and `olang
./analyze` run those files.

## The REPL

Running `olang` with no arguments starts a read-eval-print loop. Each
expression is evaluated and its result printed. The REPL accepts the same
language as a program file and keeps bindings across lines within a session.

```
$ olang
olang> let x = 21
olang> x * 2
42
olang> :help
```

Commands beginning with a colon control the session; `:help` lists them, and
`:type <expr>` reports an expression's inferred type. Type `:quit` or press
Control-D to exit.

## The browser playground

The language also runs in a web browser. The project's website compiles the
interpreter, the bytecode tier, and the full data stack to WebAssembly and
runs them sandboxed in the page. To run the website locally:

```bash
cd website
bun run dev
```

Then open the `/playground` route. Code entered in the playground executes
entirely in the browser; nothing is sent to a server. The browser build is
described in [olang in the browser](wasm.md).

## Editor support

olang includes a language server that provides diagnostics, completions,
hover information, go-to-definition, and formatting over the Language Server
Protocol. Extensions are available for Visual Studio Code and Zed, and the
server works with any LSP-capable editor. See [Editor support](editors.md)
for setup.

## Next steps

With olang installed, continue to [A tour of olang](tour.md), which teaches
the language by building a complete program.
