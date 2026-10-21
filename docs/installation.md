# Installation

This chapter installs `olang` (the language: run, check, test, format,
build) and `otc` (the project tool), and covers running programs and the
REPL.

Part of [the olang book](README.md).

## Table of contents

- [The installer script](#the-installer-script)
- [With Cargo](#with-cargo)
- [Installing from source](#installing-from-source)
- [Platforms](#platforms)
- [Updating](#updating)
- [Running programs](#running-programs)
- [The REPL](#the-repl)
- [Editor support](#editor-support)
- [Next steps](#next-steps)

## The installer script

The installer downloads a prebuilt binary from the latest release,
verifies its checksum, and installs `olang` and `otc` into
`~/.olang/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh
```

The script prints the `PATH` line to add if the directory is not already
on the path. `OLANG_VERSION=vX.Y.Z` (any release tag) pins a specific release;
`OLANG_INSTALL_DIR` changes the destination. On a platform with no
prebuilt tarball the script falls back to building from source, which
requires a Rust toolchain.

## With Cargo

For users with a Rust toolchain, Cargo installs straight from the
repository without a clone:

```bash
cargo install --locked --git https://github.com/ooyeku/olang.git olang otc
```

Binaries land on the Cargo binary path (typically `~/.cargo/bin`). A
binary built this way does not carry the browser runtime, so
`runtime.wasm()` and the web SDK's client bundle report it missing; the
installer's prebuilt binaries and a source build with `cargo xtask
install` carry it.

## Installing from source

Building from a clone requires a recent stable Rust toolchain, available
from [rustup.rs](https://rustup.rs). No other dependencies are required;
the SQLite library used by the `db` module is bundled and built from
source.

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
rustup target add wasm32-unknown-unknown   # once, for the browser runtime
cargo xtask install
```

`cargo xtask install` builds the browser runtime
([olang in the browser](wasm.md)), then installs `olang` and `otc` with
it embedded. `cargo install --path .` installs `olang` alone, without the
runtime unless `cargo xtask wasm` ran first.

Confirm any installation with:

```bash
olang --version
```

For work on the implementation itself, `cargo build --release` leaves the
binary at `target/release/olang`; run `cargo xtask wasm` once beforehand so
it embeds the browser runtime.

## Platforms

olang supports macOS, with release builds for Apple silicon and Intel.
Linux and Windows are unsupported previews: Linux binaries are released
but carry no compatibility promise, and the Windows build compiles but is
unverified. [Stability and compatibility](stability.md#platforms) states
what is promised.

A source build on Linux with the default features (`gui` among them)
needs `pkg-config libssl-dev libfontconfig1-dev libxkbcommon-dev
libwayland-dev` (Debian and Ubuntu names). Linux windows are drawn under
Wayland only; under X11 `gui.available()` is false.

## Updating

However olang was installed, `otc update --check` reports whether a
newer release exists. For installations managed under `~/.olang`,
`otc update` downloads, verifies, and switches to the latest release;
multiple releases can be kept side by side and switched with
`otc toolchain` ([Packages](packages.md#updating-and-toolchains)).
Cargo installations update by re-running the `cargo install` command.

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

The line editor completes REPL commands, functions, identifiers, and file
paths with Tab. It hints the closing brackets for whatever is still open:
typing `(map [1, 2` shows `])` as ghost text at the end of the line, and the
Right arrow accepts it. Unbalanced input continues onto the next line
with a prompt that shows which delimiters are open. When the cursor
sits on a bracket, its partner is underlined. History persists across
sessions in `~/.olang/state/history`.

Commands beginning with a colon control the session; `:help` lists them,
`:type <expr>` reports an expression's type, and `:time <expr>` wall-clocks
one evaluation. `it` always holds the last printed result. A mistyped
command suggests the nearest real one, and `:help <module>` opens any module — including
`collections` and its submodules (`:help collections.heap.push`). Type
`:quit` or press Control-D to exit.

## Editor support

olang includes a language server that provides diagnostics, completions,
hover information, go-to-definition, and formatting over the Language Server
Protocol. Extensions are available for Visual Studio Code and Zed, and the
server works with any LSP-capable editor. See [Editor support](editors.md)
for setup.

## Next steps

With olang installed, continue to [A tour of olang](tour.md), which teaches
the language by building a complete program.
