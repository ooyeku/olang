# Installation

This chapter installs `olang` (the language: run, check, test, format,
build) and `otc` (the project tool), and covers running programs and the
REPL.

Part of [the olang book](README.md).

## Table of contents

- [Install olang](#install-olang)
- [Homebrew](#homebrew)
- [Downloading a release by hand](#downloading-a-release-by-hand)
- [With Cargo](#with-cargo)
- [Installing from source](#installing-from-source)
- [Platforms](#platforms)
- [Troubleshooting](#troubleshooting)
- [Updating](#updating)
- [Uninstalling](#uninstalling)
- [Running programs](#running-programs)
- [The REPL](#the-repl)
- [Editor support](#editor-support)
- [Next steps](#next-steps)

## Install olang

On a Mac, open the Terminal app (in Applications, then Utilities), paste
this line, and press Return:

```bash
curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh
```

The installer downloads the ready-made olang for your Mac (Apple
silicon or Intel), checks it against the release's published
checksums, puts `olang` and `otc` in `~/.olang/bin`, and adds that
folder to your `PATH` in your shell's startup file (`~/.zshrc` for the
Mac's default shell). It needs no Rust, no Xcode and no administrator
password. When it finishes, **open a new Terminal window** and run:

```bash
olang --version
```

Running the installer again updates olang in place and adds nothing
twice. A few settings change what it does; put them between the `|`
and `sh`:

| Setting | Effect |
|---|---|
| `OLANG_VERSION=v1.0.0` | install that release instead of the newest |
| `OLANG_INSTALL_DIR=/some/dir` | install there instead of `~/.olang/bin` |
| `OLANG_NO_MODIFY_PATH=1` | leave startup files alone; print the line to add |
| `OLANG_FROM_SOURCE=1` | build from source even where a download exists |

For example `curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | OLANG_VERSION=v1.0.0 sh`.

On a computer with no ready-made download (see [Platforms](#platforms)),
the installer says why and offers to build olang from source, which
needs Rust; it offers to install Rust first if it is missing. On
Windows it explains how to use olang through WSL.

## Homebrew

With [Homebrew](https://brew.sh):

```bash
brew install ooyeku/olang/olang
```

That one line adds the `ooyeku/olang` tap, trusts its formula and
installs `olang` and `otc`; `brew upgrade olang` updates them. Use the
full name `ooyeku/olang/olang`: recent Homebrew refuses a formula from a
tap it has not been told to trust, and a plain `brew install olang`
after `brew tap ooyeku/olang` stops with *Refusing to load formula
... from untrusted tap* (run `brew trust ooyeku/olang` once to allow
it).

## Downloading a release by hand

Every [release](https://github.com/ooyeku/olang/releases) carries a
`.tar.gz` per platform (`macos-arm64` for Apple silicon, `macos-x64` for
Intel) and a `SHA256SUMS` file. A file downloaded with a web browser is
marked by macOS as coming from the internet, and Gatekeeper refuses to
run the programs inside, which are not notarized by Apple: *"olang" cannot be opened because
Apple cannot check it for malicious software* (or *"olang" Not Opened*).
Clear the mark on the two programs, then move them onto your `PATH`:

```bash
cd ~/Downloads/olang-*-macos-arm64
xattr -d com.apple.quarantine olang otc
mkdir -p ~/.olang/bin && mv olang otc ~/.olang/bin/
```

The installer avoids this step: `curl` does not set the mark.

## With Cargo

For users with Rust 1.97 or newer (`rustup update stable` brings an
older toolchain up to date), Cargo installs straight from the
repository without a clone:

```bash
cargo install --locked --git https://github.com/ooyeku/olang.git olang otc
```

The build takes several minutes. Binaries land on the Cargo binary path
(typically `~/.cargo/bin`, which rustup puts on `PATH`). The two
warnings about yanked packages in `Cargo.lock` are harmless. A binary
built this way does not carry the browser runtime, so `runtime.wasm()`
and the web SDK's client bundle report it missing; the installer's
prebuilt binaries and a source build with `cargo xtask install` carry
it.

## Installing from source

Building from a clone requires Rust 1.97 or newer, installed with
[rustup](https://rustup.rs). No other dependencies are required on
macOS; the SQLite library used by the `db` module is bundled and built
from source.

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
cargo xtask install
```

`cargo xtask install` adds the `wasm32-unknown-unknown` target through
rustup if it is missing, builds the browser runtime
([olang in the browser](wasm.md)), then installs `olang` and `otc` with
it embedded (`make install` does the same). `cargo install --path .`
installs `olang` alone, without the runtime unless `cargo xtask wasm`
ran first. A Rust installed without rustup (Homebrew's, for one) has no
wasm target; use `cargo install --path .` there.

For work on the implementation itself, `cargo build --release` leaves the
binary at `target/release/olang`; run `cargo xtask wasm` once beforehand so
it embeds the browser runtime.

## Platforms

olang supports macOS, with release builds for Apple silicon (macOS 11
or later) and Intel (macOS 10.12 or later). The installer installs the
Apple silicon build even from a Terminal running under Rosetta.

Linux and Windows are unsupported previews ([Stability and
compatibility](stability.md#platforms) states what is promised):

- **Linux**: binaries are released for x86-64 and arm64 and need glibc
  2.39 or newer (Ubuntu 24.04, Debian 13, Fedora 40 or later). On an
  older or musl-based (Alpine) system, or another architecture, the
  installer offers a source build instead. A source build with the
  default features (`gui` among them) needs `build-essential pkg-config
  libssl-dev libfontconfig1-dev libxkbcommon-dev libwayland-dev` (Debian
  and Ubuntu names). Linux windows are drawn under Wayland only; under
  X11 `gui.available()` is false.
- **Windows**: there is no Windows download. Install
  [WSL](https://learn.microsoft.com/windows/wsl/install) (`wsl --install`
  in PowerShell, then restart), open the Ubuntu app it adds, and run the
  installer there. The installer command does not work in PowerShell or
  Command Prompt: there `curl` is a different program, and the line
  fails with errors such as *A parameter cannot be found that matches
  parameter name 'fsSL'*.

## Troubleshooting

**`command not found: olang` right after installing.** The terminal
window that ran the installer still has the old `PATH`. Open a new
window. If a new window still cannot find it, add this line to
`~/.zshrc` (or `~/.bash_profile` for bash), then open a new window:

```bash
export PATH="$HOME/.olang/bin:$PATH"
```

**`olang --version` shows an older version.** Another olang comes
first on `PATH`, usually one from `cargo install`; `which -a olang`
lists them all. Remove the old one with `cargo uninstall olang otc`.

**Permission denied writing `~/.olang/bin`.** An earlier install ran
with `sudo` and left the folder owned by the administrator. The
installer never needs `sudo`; give the folder back with `sudo chown -R
"$(whoami)" ~/.olang` and run it again without `sudo`.

**`rustc 1.9x is not supported` or `requires rustc 1.97 or newer`.** A
source build needs a newer Rust. Run `rustup update stable`, then the
command again. A Rust from Homebrew updates with `brew upgrade rust`.

**`` version `GLIBC_2.39' not found ``** (Linux). The released Linux
binaries need a newer Linux; build from source instead (`... | OLANG_FROM_SOURCE=1 sh`).

**`libfontconfig.so.1: cannot open shared object file`** (Linux). A
server or minimal install leaves out fontconfig, which olang 1.0.0-rc.5's
Linux build links. The installer offers to install it; by hand it is
`sudo apt install libfontconfig1` (Debian, Ubuntu) or
`sudo dnf install fontconfig` (Fedora). Later releases load it only when
a window looks for system fonts.

The installer stops with a sentence that names the problem and the next
step. When one does not help, report it at
[github.com/ooyeku/olang/issues](https://github.com/ooyeku/olang/issues)
with what the installer printed.

## Updating

However olang was installed, `otc update --check` reports whether a
newer release exists. For installations managed under `~/.olang`,
`otc update` downloads, verifies, and switches to the latest release;
multiple releases can be kept side by side and switched with
`otc toolchain` ([Packages](packages.md#updating-and-toolchains)).
Running the installer again also updates. Homebrew installations update
with `brew upgrade olang`, Cargo installations by re-running the `cargo
install` command.

## Uninstalling

Delete `~/.olang/bin` (or all of `~/.olang`, which also holds packages,
REPL history and toolchains), and remove the line the installer added
to your shell's startup file; it follows the comment `# olang (added by
the olang installer)`. Homebrew: `brew uninstall olang`. Cargo: `cargo
uninstall olang otc`.

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
