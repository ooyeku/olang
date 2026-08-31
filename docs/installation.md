# Installation

This chapter describes how to install the olang compiler, run programs, use
the interactive REPL, and run the language in the browser without installing
anything.

Part of [the olang book](README.md).

## The installer script

The fastest path on macOS and Linux downloads a prebuilt binary from the
latest release, verifies its checksum, and installs `olang` and `otc`
into `~/.olang/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh
```

The script prints the `PATH` line to add if the directory is not already
on the path. `OLANG_VERSION=v0.79.0` pins a specific release;
`OLANG_INSTALL_DIR` changes the destination. On a platform with no
prebuilt tarball the script falls back to building from source, which
requires a Rust toolchain.

## Homebrew

```bash
brew install ooyeku/olang/olang
```

The tap installs both `olang` and `otc` and follows releases;
`brew upgrade olang` picks up new versions.

## With Cargo

For users with a Rust toolchain, Cargo installs straight from the
repository without a clone:

```bash
cargo install --locked --git https://github.com/ooyeku/olang.git olang otc
```

Binaries land on the Cargo binary path (typically `~/.cargo/bin`).

## Installing from source

Building from a clone requires a recent stable Rust toolchain, available
from [rustup.rs](https://rustup.rs). No other dependencies are required;
the SQLite library used by the `db` module is bundled and built from
source.

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
cargo install --path .
```

Confirm any installation with:

```bash
olang --version
```

To build without installing — for development on the compiler itself — use
`cargo build --release`; the binary is then at `target/release/olang`.

olang supports macOS and Linux, on both x86-64 and ARM. There is no
native Windows build; on Windows, use WSL and follow the Linux path.

## Updating

However olang was installed, `otc update --check` reports whether a
newer release exists. For installations managed under `~/.olang`,
`otc update` downloads, verifies, and switches to the latest release;
multiple releases can be kept side by side and switched with
`otc toolchain` ([Packages](packages.md#updating-and-toolchains)).
Homebrew installations update with `brew upgrade olang`, and Cargo
installations by re-running the `cargo install` command.

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

The line editor completes identifiers and REPL commands with Tab, and
hints the closing brackets for whatever is still open — typing
`(map [1, 2` shows `])` as ghost text at the end of the line, and the
Right arrow accepts it. Unbalanced input continues onto the next line
with a prompt that shows which delimiters are open. When the cursor
sits on a bracket, its partner is underlined. History persists across
sessions in `~/.olang/state/history`.

Commands beginning with a colon control the session; `:help` lists them,
`:type <expr>` reports an expression's type, and `:time <expr>` wall-clocks
one evaluation. `it` always holds the last printed result. TAB completes
commands, functions, and file paths; a mistyped command suggests the
nearest real one, and `:help <module>` opens any module — including
`collections` and its submodules (`:help collections.heap.push`). Type
`:quit` or press Control-D to exit.

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
