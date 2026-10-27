#!/bin/sh
# Publish olang to crates.io as `olangc` (the `olang` name there belongs
# to someone else) with its numerical engine as `olangc-ods`. The
# committed tree is copied and renamed by tools/crates_package.py; the
# repository's own manifests stay `olang` and `olang-ods`. The command
# it installs is still `olang`. Pass --dry-run to check without uploading.
set -eu
cd "$(dirname "$0")/.."
out="${TMPDIR:-/tmp}/olangc-publish"
python3 tools/crates_package.py "$PWD" "$out"
cd "$out"
cargo publish -j 4 -p olangc-ods -p olangc --allow-dirty "$@"
