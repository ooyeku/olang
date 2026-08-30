#!/usr/bin/env bash
# The Linux verification pass (roadmap W7). Runs the full gate set in a
# Linux container against this working tree: the workspace test suite,
# then the examples harness against the freshly built binary. Requires
# a running Docker-compatible daemon; the build is containerized, so
# nothing on the host is touched and the host target/ directory is not
# shared (the container builds into its own scratch volume).
#
#   dist/linux-verify.sh              # full pass
#   dist/linux-verify.sh cargo test --release --test typed_list_test
#                                     # any single command instead
#
# The pass is architecture-honest: on Apple silicon the container is
# linux/aarch64; run it on an x86_64 host (or with --platform) for
# x86_64 coverage.
set -euo pipefail
cd "$(dirname "$0")/.."

IMAGE="${OLANG_LINUX_IMAGE:-rust:1-bookworm}"
CACHE_VOLUME="${OLANG_LINUX_CACHE:-olang-linux-target}"

if [ "$#" -gt 0 ]; then
    CMD=$*
else
    CMD='cargo test --release -j 2 2>&1 | tail -30 \
         && install -m 0755 /cargo-target/release/olang /usr/local/bin/olang \
         && cd examples && olang run_all.ol'
fi

exec docker run --rm \
    -v "$(pwd)":/src \
    -v "$CACHE_VOLUME":/cargo-target \
    -e CARGO_TARGET_DIR=/cargo-target \
    -w /src \
    "$IMAGE" \
    bash -c "rustc --version && uname -sm && $CMD"
