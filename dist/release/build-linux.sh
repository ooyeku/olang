#!/bin/sh
# Builds one Linux release tarball inside a container. The release
# workflow (.github/workflows/release.yml) runs it in two kinds:
#
#   manylinux_2_28 (AlmaLinux 8, glibc 2.28): the standard build. Built
#     against an old glibc, it runs on Linux from 2019 on: Debian 10,
#     Ubuntu 20.04, RHEL and Rocky 8, Amazon Linux 2023, Raspberry Pi OS
#     (64-bit) and newer.
#   Alpine: the static musl build. It needs nothing from the system, so it
#     runs on Alpine and on Linux too old for the standard build. Windows
#     (the gui module) need Wayland loaded at run time, which a static
#     program cannot do, so this build leaves gui out: gui.available() is
#     false there.
#
#   dist/release/build-linux.sh VERSION NAME     (NAME: linux-x64, linux-arm64-musl, ...)
#
# Run from the repository root. Leaves olang-VERSION-NAME.tar.gz there.
set -eu

VERSION=$1
NAME=$2
STAGE="olang-$VERSION-$NAME"
MAX_GLIBC="2.28"

if [ -f /etc/alpine-release ]; then
    apk add --no-cache build-base linux-headers perl cmake curl
fi
if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y --profile minimal --default-toolchain stable
    PATH="$HOME/.cargo/bin:$PATH"
fi
rustup target add wasm32-unknown-unknown
rustc --version

# The browser runtime the olang binary embeds (build.rs reads it).
cargo xtask wasm

case "$NAME" in
    *-musl)
        cargo build --release --locked -p olang -p otc \
            --no-default-features --features olang/native,olang/full ;;
    *)
        cargo build --release --locked -p olang -p otc ;;
esac

rm -rf "$STAGE"
mkdir "$STAGE"
cp target/release/olang target/release/otc LICENSE README.md "$STAGE/"
strip "$STAGE/olang" "$STAGE/otc"

# Check the promise each build makes before shipping it.
for bin in olang otc; do
    case "$NAME" in
        *-musl)
            # static: no program interpreter and no shared libraries
            if readelf -l "$STAGE/$bin" | grep -q INTERP || readelf -d "$STAGE/$bin" | grep -q NEEDED; then
                echo "$bin is not static:" >&2
                readelf -d "$STAGE/$bin" >&2
                exit 1
            fi ;;
        *)
            newest=$(objdump -T "$STAGE/$bin" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -t. -k1,1n -k2,2n | tail -n 1)
            echo "$bin needs glibc $newest"
            if [ "$(printf '%s\n%s\n' "$newest" "$MAX_GLIBC" | sort -t. -k1,1n -k2,2n | tail -n 1)" != "$MAX_GLIBC" ]; then
                echo "$bin needs glibc $newest, newer than the $MAX_GLIBC this build promises" >&2
                exit 1
            fi ;;
    esac
    "./$STAGE/$bin" --version
done

tar -czf "$STAGE.tar.gz" "$STAGE"
sha256sum "$STAGE.tar.gz"
