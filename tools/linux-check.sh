#!/usr/bin/env bash
# linux-check — build olang (with `gui`) on Linux in a container and run
# the headless checks against it, locally (Docker or OrbStack; arm64 or
# x86_64 Linux, whichever the machine runs natively). No GitHub Actions.
#
#   tools/linux-check.sh [stage ...]
#
# Stages (all of them when none is named, in this order):
#   build    cargo build --release -j 4 (olang's lib and bin)
#   loom     Loom's tools/verify.sh with OLANG = the Linux binary
#   pty      Loom's terminal face in a real pseudo-terminal (tools/pty_check.py)
#   app      open-track-desktop's tools/verify.sh, same binary
#   guitest  cargo test -j 4 --test gui_test (debug)
#   heddle   Heddle's suite (olang test)
#   baseline Loom's benchmark gate re-recorded for Linux
#            (loom/tools/bench_baseline.linux.json; not in the default run)
#   shell    an interactive shell in the same container setup
#
# The sibling repos are found next to olang (OLANG_WORLD, default the
# parent of this checkout): loom, open-track-desktop, heddle. They are
# mounted read-write at /w (the suites write .build/ logs and snapshots'
# .actual/.diff files there). Cargo's target and registry live in the
# named volume $OLANG_LINUX_VOLUME (default olang-linux-target), so the
# Linux build never touches olang/target on the host; `docker volume rm
# olang-linux-target` reclaims it.
#
# The container is limited (LINUX_CHECK_CPUS, default 4; LINUX_CHECK_MEM,
# default 8g), runs with --rm, and only one stage builds at a time: never
# run this while a host cargo build is going.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
world="${OLANG_WORLD:-$(dirname "$here")}"
volume="${OLANG_LINUX_VOLUME:-olang-linux-target}"
image="olang-linux-check"
cpus="${LINUX_CHECK_CPUS:-4}"
mem="${LINUX_CHECK_MEM:-8g}"

# The image: Rust's official one plus what olang's Linux build links or
# loads (the list .github/workflows install: OpenSSL, fontconfig,
# xkbcommon, Wayland), python3 for Loom's pty check, procps (pgrep) for
# Heddle's process tests, and Mesa's software Vulkan (lavapipe) so
# gui_test's GPU-against-software comparisons run rather than skip. The
# image is built once; LINUX_CHECK_REBUILD_IMAGE=1 builds it again.
if [ -z "$(docker images -q "$image" 2>/dev/null)" ] || [ "${LINUX_CHECK_REBUILD_IMAGE:-0}" = 1 ]; then
  docker build -t "$image" - <<'EOF'
FROM rust:1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config libssl-dev libfontconfig1-dev libxkbcommon-dev libwayland-dev python3 procps \
      libvulkan1 mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*
EOF
fi

run() {
  local tty=()
  [ -t 0 ] && [ -t 1 ] && tty=(-it)
  docker run --rm ${tty[@]+"${tty[@]}"} --cpus "$cpus" --memory "$mem" \
    -v "$world:/w" -v "$volume:/cache" \
    -e CARGO_TARGET_DIR=/cache/target -e CARGO_HOME=/cache/cargo-home \
    -e PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    -e OLANG=/cache/target/release/olang \
    -w /w "$image" bash -c "$1"
}

[ $# -eq 0 ] && set -- build loom pty app guitest heddle
status=0
for s in "$@"; do
  echo "== $s"
  case "$s" in
    build)   run 'cd olang && cargo build --release -j 4' || status=1 ;;
    loom)    run 'cd loom && tools/verify.sh' || status=1 ;;
    pty)     run 'cd loom && python3 tools/pty_check.py' || status=1 ;;
    app)     run 'cd open-track-desktop && tools/verify.sh' || status=1 ;;
    guitest) run 'cd olang && cargo test -j 4 --test gui_test' || status=1 ;;
    heddle)  run 'cd heddle && $OLANG test' || status=1 ;;
    baseline) run 'cd loom && $OLANG tools/gate.ol --update' || status=1 ;;
    shell)   run 'bash' ;;
    *) echo "unknown stage: $s" >&2; exit 2 ;;
  esac
done
exit $status
