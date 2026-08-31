#!/bin/sh
# The olang installer.
#
#   curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh
#
# Detects the platform, downloads the matching tarball from the latest
# GitHub release (or the release named in OLANG_VERSION, e.g. "v0.79.0"),
# verifies it against the release's SHA256SUMS, and installs `olang` and
# `otc` into ~/.olang/bin (override with OLANG_INSTALL_DIR). When no
# prebuilt tarball matches the platform, it falls back to building from
# source, which requires a Rust toolchain.
#
# The script is POSIX sh and safe to re-run; an existing install is
# replaced in place.
set -eu

REPO="ooyeku/olang"
INSTALL_DIR="${OLANG_INSTALL_DIR:-$HOME/.olang/bin}"
VERSION="${OLANG_VERSION:-}"

say()  { printf '%s\n' "$*"; }
fail() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || fail "curl is required"

# ── platform ─────────────────────────────────────────────────────────
os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Darwin) case "$arch" in
                arm64)  platform="macos-arm64" ;;
                x86_64) platform="macos-x64" ;;
                *)      platform="" ;;
            esac ;;
    Linux)  case "$arch" in
                x86_64)          platform="linux-x64" ;;
                aarch64|arm64)   platform="linux-arm64" ;;
                *)               platform="" ;;
            esac ;;
    *)      platform="" ;;
esac

# ── resolve the release ──────────────────────────────────────────────
if [ -z "$VERSION" ]; then
    VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
        sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$VERSION" ] || fail "could not determine the latest release (set OLANG_VERSION to pin one)"
fi
version_no_v="${VERSION#v}"
base="https://github.com/$REPO/releases/download/$VERSION"

# ── source-build fallback ────────────────────────────────────────────
build_from_source() {
    say "No prebuilt binary for $os/$arch — building from source (this needs Rust and a few minutes)."
    command -v cargo >/dev/null 2>&1 || fail "cargo not found; install Rust from https://rustup.rs and re-run"
    cargo install --locked --git "https://github.com/$REPO.git" --tag "$VERSION" olang otc ||
        fail "source build failed"
    say "Installed olang and otc via cargo (into ~/.cargo/bin)."
    exit 0
}
[ -n "$platform" ] || build_from_source

# ── download and verify ──────────────────────────────────────────────
tarball="olang-$version_no_v-$platform.tar.gz"
workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

say "Downloading $tarball ($VERSION)..."
curl -fsSL -o "$workdir/$tarball" "$base/$tarball" ||
    fail "download failed: $base/$tarball (release still building? platform unreleased?)"
curl -fsSL -o "$workdir/SHA256SUMS" "$base/SHA256SUMS" ||
    fail "download failed: $base/SHA256SUMS"

expected=$(sed -n "s/^\([0-9a-f]\{64\}\)  *$tarball\$/\1/p" "$workdir/SHA256SUMS")
[ -n "$expected" ] || fail "$tarball is not listed in SHA256SUMS"
if command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$workdir/$tarball" | cut -d' ' -f1)
else
    actual=$(sha256sum "$workdir/$tarball" | cut -d' ' -f1)
fi
[ "$actual" = "$expected" ] || fail "checksum mismatch for $tarball (expected $expected, got $actual)"

# ── install ──────────────────────────────────────────────────────────
tar -xzf "$workdir/$tarball" -C "$workdir"
mkdir -p "$INSTALL_DIR"
for bin in olang otc; do
    cp "$workdir/olang-$version_no_v-$platform/$bin" "$INSTALL_DIR/$bin.new"
    chmod +x "$INSTALL_DIR/$bin.new"
    mv -f "$INSTALL_DIR/$bin.new" "$INSTALL_DIR/$bin"
done

say "Installed olang $version_no_v to $INSTALL_DIR"
"$INSTALL_DIR/olang" --version

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        say ""
        say "Add olang to your PATH:"
        say "    export PATH=\"$INSTALL_DIR:\$PATH\""
        say "(append that line to your shell profile, e.g. ~/.zshrc or ~/.bashrc)"
        ;;
esac
