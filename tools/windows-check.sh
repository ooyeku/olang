#!/usr/bin/env bash
# windows-check — check that olang (with `gui`) compiles for Windows, from
# macOS or Linux, locally and with nothing from Microsoft (no Windows SDK,
# no MSVC CRT, no xwin).
#
#   tools/windows-check.sh            # cargo check --target x86_64-pc-windows-msvc
#   tools/windows-check.sh --link     # and build olang.exe for x86_64-pc-windows-gnu,
#                                     # linked by zig against MinGW
#
# Needs zig on the path and the Rust targets (rustup target add
# x86_64-pc-windows-msvc, and x86_64-pc-windows-gnu for --link). `cargo
# check` still runs the build scripts, and three of them compile C for
# the target (SQLite, aws-lc for rustls, psm for the stack guard); zig's
# clang with its bundled MinGW headers does it, so the C side is compiled
# as for windows-gnu while Rust checks against msvc: enough for a check,
# never for an msvc link. The link is the gnu target's.
#
# It builds into target/windows-check (not the host's target/debug), at
# -j 4. Never run it while another cargo build is going.
set -euo pipefail
cd "$(dirname "$0")/.."
target=x86_64-pc-windows-msvc
gnu=x86_64-pc-windows-gnu
link=0
if [ "${1:-}" = "--link" ]; then link=1; shift; fi
command -v zig >/dev/null || { echo "windows-check: zig is not on the path" >&2; exit 2; }
installed="$(rustup target list --installed)"
echo "$installed" | grep -qx "$target" || { echo "windows-check: rustup target add $target" >&2; exit 2; }
if [ "$link" = 1 ]; then
  echo "$installed" | grep -qx "$gnu" || { echo "windows-check: rustup target add $gnu" >&2; exit 2; }
fi

export CARGO_TARGET_DIR="$PWD/target/windows-check"
shim="$CARGO_TARGET_DIR/zig-shim"
mkdir -p "$shim"
# zig as the C compiler and the linker. cc-rs adds --target=<rust triple>,
# which zig does not parse: it is dropped, zig's own -target names the
# triple. rustc brackets libraries with -Bstatic/-Bdynamic, and zig then
# looks for a .dll where MinGW has lib*.a import libraries: dropped too,
# as is -lmsvcrt (zig links its own MinGW C runtime).
printf '%s\n' '#!/bin/sh' \
  'for a; do shift; case "$a" in --target=*|-Wl,-Bstatic|-Wl,-Bdynamic|-lmsvcrt|-l:libpthread.a) ;; *) set -- "$@" "$a" ;; esac; done' \
  'exec zig cc -target x86_64-windows-gnu "$@"' > "$shim/cc"
# For an msvc target cc-rs archives as lib.exe would (-out:, -nologo):
# zig's llvm-lib takes those. For gnu, ar.
printf '%s\n' '#!/bin/sh' 'exec zig lib "$@"' > "$shim/lib"
printf '%s\n' '#!/bin/sh' 'exec zig ar "$@"' > "$shim/ar"
# rustc builds the import libraries of the `windows` crates' raw-dylib
# with MinGW's dlltool, by name: zig's llvm-dlltool stands in.
# (On a PATH of its own: the host's cc must stay the host's.)
mkdir -p "$shim/path"
printf '%s\n' '#!/bin/sh' 'exec zig dlltool "$@"' > "$shim/path/x86_64-w64-mingw32-dlltool"
chmod +x "$shim/cc" "$shim/lib" "$shim/ar" "$shim/path/x86_64-w64-mingw32-dlltool"

echo "== cargo check --target $target"
CC_x86_64_pc_windows_msvc="$shim/cc" CXX_x86_64_pc_windows_msvc="$shim/cc" \
AR_x86_64_pc_windows_msvc="$shim/lib" \
  cargo check -j 4 --target "$target" "$@"
[ "$link" = 1 ] || exit 0

echo "== cargo build --target $gnu --bin olang (zig links)"
CC_x86_64_pc_windows_gnu="$shim/cc" CXX_x86_64_pc_windows_gnu="$shim/cc" \
AR_x86_64_pc_windows_gnu="$shim/ar" CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$shim/cc" \
PATH="$shim/path:$PATH" cargo build -j 4 --target "$gnu" --bin olang "$@"
ls -l "$CARGO_TARGET_DIR/$gnu/debug/olang.exe"
