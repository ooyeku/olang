# Installation

This page gets olang onto your computer. You get two programs: `olang`,
the language itself (it runs, checks, tests and formats your code), and
`otc`, which creates projects and manages packages.

Part of [the olang book](README.md).

## Table of contents

- [Install olang](#install-olang)
- [Updating](#updating)
- [Uninstalling](#uninstalling)
- [Other ways to install](#other-ways-to-install)
- [Platforms](#platforms)
- [How downloads are checked](#how-downloads-are-checked)
- [Troubleshooting](#troubleshooting)
- [Running programs](#running-programs)
- [The REPL](#the-repl)
- [Editor support](#editor-support)
- [Next steps](#next-steps)

## Install olang

On a Mac or on Linux, open a terminal (on a Mac, the Terminal app is in
Applications, then Utilities), paste this line and press Return:

```bash
curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh
```

On Windows, open PowerShell and run:

```powershell
irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex
```

The installer works out what kind of computer you have, downloads the
matching olang, checks that the download is genuine, and puts `olang`
and `otc` in a folder in your home directory (`~/.olang/bin`). It adds
that folder to your `PATH`, the list of places your terminal looks for
programs. It needs no administrator password and no Rust.

A terminal only reads that list when it opens, so the window you
installed from doesn't know about olang yet. When the installer
finishes, either quit and reopen your terminal, or bring the change
into the window you have with `source` and the file the installer names
(for zsh, the usual shell on a Mac, that is `source ~/.zshrc`). Then run:

```bash
olang --version
```

That's it. Head to [A tour of olang](tour.md) next.

Running the installer again updates olang to the newest release, and
does nothing at all when you already have it. The same files are also
at `https://raw.githubusercontent.com/ooyeku/olang/main/install.sh` (and
`install.ps1`) if you would rather fetch them from GitHub directly.

### Choosing what it does

The installer takes a few options. Put them after `sh -s --`:

```bash
curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh -s -- --version v1.0.0
```

| Option | Environment variable | Effect |
|---|---|---|
| `--version v1.0.0` | `OLANG_VERSION` | install that release instead of the newest |
| `--dir DIR` | `OLANG_INSTALL_DIR` | install into `DIR` instead of `~/.olang/bin` |
| `--no-modify-path` | `OLANG_NO_MODIFY_PATH=1` | leave your shell's startup files alone and print the line to add |
| `--from-source` | `OLANG_FROM_SOURCE=1` | build from source even where a download exists |
| `--force` | `OLANG_FORCE=1` | reinstall even when this version is already there |
| `-y`, `--yes` | `OLANG_YES=1` | answer yes to every question (installing Rust, a missing system package) |
| `-q`, `--quiet` | `OLANG_QUIET=1` | print nothing but errors, for CI and scripts |
| `--uninstall` | | remove olang (see [Uninstalling](#uninstalling)) |
| `--help` | | list the options |

On Windows, set the environment variable first, for example
`$env:OLANG_VERSION = 'v1.0.0'; irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex`.

In CI, a typical line is:

```bash
curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh -s -- --quiet --no-modify-path
echo "$HOME/.olang/bin" >> "$GITHUB_PATH"
```

### What the installer touches

It writes the two programs into the install folder, adds one line to
your shell's startup file (`~/.zshrc` for zsh, the Mac's default;
`~/.bashrc` for bash on Linux; a file in `~/.config/fish/conf.d` for
fish), and deletes its temporary files when it ends. The line it adds
sits under the comment `# olang (added by the olang installer)`. It
never runs `sudo` on its own. The one time it might want to, to install
a missing system library on Linux, it shows you the exact command and
asks first.

Where no ready-made download fits your computer, the installer tells
you why and offers to build olang from source, which needs Rust. It
offers to install Rust too, and asks before doing either.

## Updating

Run the install command again. It replaces olang in place with the
newest release. `otc update` does the same from inside olang, and
`otc update --check` only reports whether a newer release exists. `otc`
can also keep several releases side by side and switch between them
with `otc toolchain` ([Packages](packages.md#updating-and-toolchains)).

Homebrew users update with `brew upgrade olang`, and Cargo users by
running their `cargo install` command again.

## Uninstalling

```bash
curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh -s -- --uninstall
```

This removes `olang` and `otc` and the `PATH` line the installer added.
Your packages, REPL history and toolchains in `~/.olang` stay, unless you
answer yes when it asks; `rm -rf ~/.olang` removes them later. On
Windows, run `$env:OLANG_UNINSTALL = '1'; irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex`.

For other installs: `brew uninstall olang`, or `cargo uninstall olang otc`
(`make remove` in a checkout does the same).

## Other ways to install

### Homebrew

With [Homebrew](https://brew.sh) on macOS or Linux:

```bash
brew install ooyeku/olang/olang
```

Use the full name. It adds olang's tap, the small GitHub repository that
holds the formula, and tells Homebrew to trust it. Recent Homebrew
refuses a formula from a tap it has not been told to trust, so `brew tap
ooyeku/olang` followed by a plain `brew install olang` stops with
*Refusing to load formula ... from untrusted tap*. (`brew trust
ooyeku/olang` allows it.) The formula pins each download's checksum, and
Homebrew checks it.

### Downloading a release by hand

Every [release](https://github.com/ooyeku/olang/releases) has a file per
platform (see [Platforms](#platforms) for which is which), a
`SHA256SUMS` file with each file's checksum, and, once releases are
signed, `SHA256SUMS.sig`, the signature on those checksums. To check a
download before you unpack it:

```bash
curl -fsSLO https://raw.githubusercontent.com/ooyeku/olang/main/dist/signing/olang-release.pub
openssl dgst -sha256 -verify olang-release.pub -signature SHA256SUMS.sig SHA256SUMS
shasum -a 256 -c SHA256SUMS --ignore-missing
```

With the GitHub CLI, `gh attestation verify olang-1.0.0-macos-arm64.tar.gz
--repo ooyeku/olang` checks that the file was built by olang's release
workflow.

On a Mac, a file downloaded with a web browser is marked as coming from
the internet, and Gatekeeper refuses to run the programs inside, because
they are not notarized by Apple: *"olang" cannot be opened because Apple
cannot check it for malicious software* (or *"olang" Not Opened*).
Clear the mark on the two programs, then move them onto your `PATH`:

```bash
cd ~/Downloads/olang-*-macos-arm64
xattr -d com.apple.quarantine olang otc
mkdir -p ~/.olang/bin && mv olang otc ~/.olang/bin/
```

The installer avoids this step, because `curl` does not set the mark.

### With Cargo

If you have Rust 1.97 or newer (`rustup update stable` brings an older
one up to date), Cargo can build and install olang straight from the
repository:

```bash
cargo install --locked --git https://github.com/ooyeku/olang.git olang otc
```

`--locked` builds with exactly the dependency versions olang was tested
with. The build takes several minutes, and the programs land in Cargo's
folder (`~/.cargo/bin`, which rustup puts on your `PATH`). Two warnings
about yanked packages in `Cargo.lock` are harmless. A program built this
way lacks the browser runtime, so `runtime.wasm()` and the web SDK's
client bundle report it missing; the ready-made downloads and
`cargo xtask install` include it.

### From source

Building from a clone needs Rust 1.97 or newer, installed with
[rustup](https://rustup.rs). On macOS nothing else is needed; the SQLite
library behind the `db` module is built from source with olang. On Linux
you also need a C compiler (`sudo apt install build-essential` on Debian
and Ubuntu).

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
cargo xtask install
```

`cargo xtask install` adds rustup's `wasm32-unknown-unknown` target if it
is missing, builds the browser runtime ([olang in the browser](wasm.md)),
then installs `olang` and `otc` with it built in. `make install` does the
same. `cargo install --path .` installs `olang` alone, without the
runtime unless `cargo xtask wasm` ran first. A Rust installed without
rustup (Homebrew's, for one) has no wasm target, so use `cargo install
--path .` there.

To work on olang itself, `cargo build --release` leaves the program at
`target/release/olang`. Run `cargo xtask wasm` once beforehand so it
includes the browser runtime.

## Platforms

olang is developed and tested on macOS, and runs on Linux and Windows
too. [Stability and compatibility](stability.md#platforms) says what is
promised on each.

| Computer | Download | Notes |
|---|---|---|
| Mac with Apple silicon | `macos-arm64` | macOS 11 or later. The installer picks it even from a Terminal running under Rosetta. |
| Intel Mac | `macos-x64` | macOS 10.12 or later |
| Linux, x86-64 | `linux-x64` | glibc 2.28 or later: Ubuntu 20.04, Debian 10, Fedora 29, RHEL 8 and anything newer. WSL works the same way. |
| Linux, ARM 64-bit | `linux-arm64` | the same distributions, and Raspberry Pi OS (64-bit) on a Pi 3, 4 or 5 |
| Alpine, or a Linux older than glibc 2.28 | `linux-x64-musl`, `linux-arm64-musl` | a static build that needs nothing from the system. It has everything except windows: `gui.available()` is false. |
| Windows 10 or 11, x64 | `windows-x64` | through `install.ps1`. Windows on ARM runs it under x64 emulation. |
| Anything else | | the installer offers to build from source |

The installer chooses the right download by itself, including the
static build on Alpine or an old Linux. A 32-bit system (a Raspberry Pi
running the 32-bit Raspberry Pi OS, say) has no download; install the
64-bit OS instead.

On Linux, olang draws windows under Wayland only; under X11
`gui.available()` is false. A server or minimal install without
fontconfig still runs olang, and its windows then use olang's bundled
fonts.

The `curl ... | sh` line is for macOS and Linux. In PowerShell or the
Command Prompt, `curl` is a different program, and the line fails with
errors such as *A parameter cannot be found that matches parameter name
'fsSL'*. Use `install.ps1` there, or run the `curl` line inside
[WSL](https://learn.microsoft.com/windows/wsl/install).

## How downloads are checked

Every download goes over HTTPS, and the installer refuses to fall back
to plain HTTP. Each release lists a checksum, a fingerprint of the file,
for every download in `SHA256SUMS`, and the installer compares the file
it downloaded against it. A checksum proves the file arrived intact, but
not who made it, since it comes from the same place. So the release
workflow also signs `SHA256SUMS` with olang's release key, and the
installer checks that signature with the public key written into the
installer itself, using `openssl` on macOS and Linux and .NET on Windows.
A file changed on the download server fails that check, and nothing is
installed.

On a Linux without `openssl` the installer still checks the checksum,
and says that it could not check the signature. Set
`OLANG_REQUIRE_SIGNATURE=1` to make that an error instead.

Every release also carries a GitHub build attestation for each file, a
signed record of the workflow run that built it, which `gh attestation
verify` checks. Homebrew pins each file's checksum in the formula, and
`cargo install --locked` builds exactly the dependencies in olang's
`Cargo.lock`.

## Troubleshooting

The installer stops with a sentence that names the problem and the next
step. These are the ones people meet most.

**`command not found: olang` right after installing.** The terminal
window that ran the installer still has the old `PATH`. Open a new
window. If a new window still cannot find it, add this line to
`~/.zshrc` (or `~/.bashrc` for bash), then open a new window:

```bash
export PATH="$HOME/.olang/bin:$PATH"
```

**`olang --version` shows an older version.** Another olang comes first
on your `PATH`, usually one from `cargo install`. `which -a olang` lists
them all; remove the old one with `cargo uninstall olang otc`.

**Permission denied writing `~/.olang/bin`.** An earlier install ran
with `sudo` and left the folder owned by the administrator. The
installer never needs `sudo`. Give the folder back with
`sudo chown -R "$(whoami)" ~/.olang` and run it again without `sudo`.

**"The installer was run with sudo".** Run it as yourself. With `sudo`
it would install olang into the administrator's home folder, where you
could not use it.

**`rustc 1.9x is not supported` or `requires rustc 1.97 or newer`.** A
source build needs a newer Rust. Run `rustup update stable`, then the
command again. A Rust from Homebrew updates with `brew upgrade rust`.

**`` version `GLIBC_2.28' not found ``** (Linux). Your Linux is older
than the standard build supports. The installer switches to the static
build by itself; if you downloaded by hand, take the `-musl` file
instead.

**`error while loading shared libraries`** (Linux). A minimal install
lacks a system library olang uses. The installer names the package and
offers to install it, for example `sudo apt install libgcc-s1`.

**The release's signature does not match.** Don't install that release
by hand. Report it at
[github.com/ooyeku/olang/issues](https://github.com/ooyeku/olang/issues),
and install an earlier one with `--version` in the meantime.

**Windows: "running scripts is disabled on this system".** That message
comes from running a saved `install.ps1`. The `irm ... | iex` line is not
affected; or run
`powershell -ExecutionPolicy Bypass -File install.ps1`.

When none of this helps, report it at
[github.com/ooyeku/olang/issues](https://github.com/ooyeku/olang/issues)
with what the installer printed.

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
