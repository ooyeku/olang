#!/bin/sh
# The olang installer.
#
#   curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh
#
# (https://raw.githubusercontent.com/ooyeku/olang/main/install.sh is the
# same file.) It downloads the ready-made olang for this computer from
# the newest GitHub release, checks the release's signature and the
# download's checksum, installs `olang` and `otc` into ~/.olang/bin, and
# adds that folder to your PATH. Where no ready-made download fits, it
# says why and offers to build olang from source, which needs Rust.
#
# Run `sh install.sh --help` for the options. Options go after `sh -s --`
# when the script is piped:
#
#   curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh -s -- --version v1.0.0
#
# What it touches: the install folder (~/.olang/bin unless you choose
# another), one line in your shell's startup file (unless you say not
# to), and a temporary folder it deletes when it ends. It never runs sudo
# without asking first.
#
# POSIX sh, tested under sh, dash and bash. The whole script is one
# function called on the last line, so a download cut short runs nothing.
set -u

REPO="ooyeku/olang"
GITHUB="${OLANG_GITHUB:-https://github.com}"
GITHUB_API="${OLANG_GITHUB_API:-https://api.github.com}"
INSTALL_URL="https://raw.githubusercontent.com/ooyeku/olang/main/install.sh"
HELP_URL="https://olang.dev/book/installation/"
# The oldest Rust that builds olang: keep equal to `rust-version` in
# Cargo.toml (tests/install_script_test.rs checks).
MIN_RUST="1.97"
# The oldest glibc the standard Linux build runs on (the release builds
# it in a manylinux_2_28 container). Older Linux gets the static build.
MIN_GLIBC="2.28"

# The public half of olang's release signing key, an ECDSA P-256 key.
# CI signs each release's SHA256SUMS with the private half, and this
# script checks that signature with `openssl` before trusting any
# checksum in it. dist/signing/README.md says how the key is made.
# Empty until signing is set up; then every release from SIGNED_FROM on
# must carry a good SHA256SUMS.sig.
SIGNING_KEY=""
SIGNED_FROM=""

# ── output ──────────────────────────────────────────────────────────
QUIET=""
setup_output() {
    if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != dumb ]; then
        BOLD=$(printf '\033[1m'); DIM=$(printf '\033[2m'); RED=$(printf '\033[31m')
        GREEN=$(printf '\033[32m'); YELLOW=$(printf '\033[33m'); RESET=$(printf '\033[0m')
    else
        BOLD=""; DIM=""; RED=""; GREEN=""; YELLOW=""; RESET=""
    fi
    case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
        *UTF-8*|*utf-8*|*UTF8*|*utf8*) TICK="✓"; CROSS="✗" ;;
        *) TICK="ok"; CROSS="!!" ;;
    esac
}
say()  { [ -n "$QUIET" ] || printf '%s\n' "$*"; }
# row LABEL VALUE: one line of the summary at the top
row()  { [ -n "$QUIET" ] || printf '  %s%-9s%s %s\n' "$DIM" "$1" "$RESET" "$2"; }
# done_ TEXT: a step that finished
done_() { [ -n "$QUIET" ] || printf '  %s%s%s %s\n' "$GREEN" "$TICK" "$RESET" "$*"; }
# note TEXT...: something worth knowing that does not stop the install
note() {
    [ -n "$QUIET" ] && return 0
    printf '  %s%s%s %s\n' "$YELLOW" "!" "$RESET" "$1"
    shift
    for line in "$@"; do printf '    %s\n' "$line"; done
}
# fail "what went wrong" "what to do" ["more"...]: the problem as a plain
# sentence, then the next step, then stop. Shown even in quiet mode.
fail() {
    printf '\n  %s%s olang was not installed:%s %s\n' "$RED$BOLD" "$CROSS" "$RESET" "$1" >&2
    shift
    [ $# -gt 0 ] && printf '\n' >&2
    for line in "$@"; do printf '    %s\n' "$line" >&2; done
    printf '\n  %sHelp: %s%s\n\n' "$DIM" "$HELP_URL" "$RESET" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }
# a path as people read it: ~/.olang/bin rather than /Users/you/.olang/bin
tilde() { case "$1" in "$HOME"/*) printf '~%s' "${1#"$HOME"}" ;; *) printf '%s' "$1" ;; esac; }

# ask "question": yes with --yes (OLANG_YES=1), no with OLANG_YES=0 or
# when there is nobody to ask (no terminal), else whatever the person
# types. Reads the terminal, not
# stdin, because under `curl ... | sh` stdin is this script.
ask() {
    [ "${OLANG_YES:-}" = 1 ] && return 0
    [ "${OLANG_YES:-}" = 0 ] && return 1
    { [ -r /dev/tty ] && [ -w /dev/tty ]; } 2>/dev/null || return 1
    printf '  %s [y/N] ' "$1" 2>/dev/null >/dev/tty || return 1
    read -r answer </dev/tty 2>/dev/null || return 1
    case "$answer" in y|Y|yes|YES|Yes) return 0 ;; *) return 1 ;; esac
}

usage() {
    cat <<EOF
The olang installer

Usage:
  curl -fsSL $INSTALL_URL | sh
  curl -fsSL $INSTALL_URL | sh -s -- [options]
  sh install.sh [options]

Options:
  --version VERSION   install this release, e.g. v1.0.0 (default: the newest)
  --dir DIR           install into DIR (default: ~/.olang/bin)
  --no-modify-path    leave your shell's startup files alone
  --from-source       build from source even where a download exists
  --force             reinstall even when this version is already installed
  -y, --yes           answer yes to every question
  -q, --quiet         print nothing but errors (for CI and scripts)
  --uninstall         remove olang and the PATH line this script added
  -h, --help          show this help

Each option also has an environment variable: OLANG_VERSION,
OLANG_INSTALL_DIR, OLANG_NO_MODIFY_PATH=1, OLANG_FROM_SOURCE=1,
OLANG_FORCE=1, OLANG_YES=1, OLANG_QUIET=1.

The installer writes to the install folder, adds one line to your
shell's startup file (~/.zshrc, ~/.bashrc, ...), and deletes its
temporary files when it ends. It downloads only over HTTPS and checks
each download against the release's signed checksums.

More: $HELP_URL
EOF
}

parse_args() {
    while [ $# -gt 0 ]; do
        case "$1" in
            -h|--help) setup_output; usage; exit 0 ;;
            -q|--quiet) OLANG_QUIET=1 ;;
            -y|--yes) OLANG_YES=1 ;;
            --uninstall) ACTION=uninstall ;;
            --no-modify-path) OLANG_NO_MODIFY_PATH=1 ;;
            --from-source) OLANG_FROM_SOURCE=1 ;;
            --force) OLANG_FORCE=1 ;;
            --version|--dir)
                [ $# -ge 2 ] || { setup_output; fail "the option $1 needs a value." "For example: $1 $([ "$1" = --dir ] && echo "\$HOME/bin" || echo v1.0.0)"; }
                if [ "$1" = --version ]; then OLANG_VERSION=$2; else OLANG_INSTALL_DIR=$2; fi
                shift ;;
            --version=*) OLANG_VERSION=${1#*=} ;;
            --dir=*) OLANG_INSTALL_DIR=${1#*=} ;;
            *) setup_output
               fail "the installer does not know the option '$1'." \
                   "Run it with --help to see the options it takes." ;;
        esac
        shift
    done
}

# ── downloads ───────────────────────────────────────────────────────
# Every download is HTTPS only (TLS 1.2 or newer, never redirected to
# plain http). curl is preferred; wget (GNU or BusyBox, as on Alpine)
# works too.
pick_downloader() {
    if have curl; then DL=curl
    elif have wget; then
        DL=wget
        # GNU wget can refuse a redirect to http; BusyBox wget has no such
        # option, but every URL here starts at https and GitHub only
        # redirects to https.
        if wget --help 2>&1 | grep -q -- --https-only; then
            WGET_SAFE="--https-only --secure-protocol=TLSv1_2"
        else
            WGET_SAFE=""
        fi
    else
        fail "this computer has neither 'curl' nor 'wget', and the installer needs one to download olang." \
            "On Debian or Ubuntu:  sudo apt install curl" \
            "On Fedora:            sudo dnf install curl" \
            "On Alpine:            apk add curl" \
            "Then run the install command again."
    fi
}

# fetch URL FILE [progress]: download URL to FILE. Returns 0, or 22 when
# the server says the file is not there, or another code for a network
# problem (see network_fail).
fetch() {
    if [ "$DL" = curl ]; then
        if [ "${3:-}" = progress ] && [ -t 2 ] && [ -z "$QUIET" ]; then
            curl --proto '=https' --tlsv1.2 -fL --progress-bar --retry 2 --connect-timeout 15 -o "$2" "$1"
        else
            curl --proto '=https' --tlsv1.2 -fsSL --retry 2 --connect-timeout 15 -o "$2" "$1"
        fi
    else
        # shellcheck disable=SC2086 # WGET_SAFE is a list of options
        wget -q $WGET_SAFE -T 15 -O "$2" "$1" 2>/dev/null
        rc=$?
        [ $rc -eq 8 ] && return 22  # the server answered with an error
        return $rc
    fi
}

# final_url URL: where URL ends up after its redirects (prints it).
final_url() {
    if [ "$DL" = curl ]; then
        curl --proto '=https' --tlsv1.2 -sL -o /dev/null -w '%{url_effective}' --connect-timeout 15 "$1"
    else
        # shellcheck disable=SC2086
        wget -S --spider $WGET_SAFE -T 15 "$1" 2>&1 | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1 | tr -d '\r'
    fi
}

# exists URL: 0 when the server has it, 22 when not, else a network code
exists() {
    if [ "$DL" = curl ]; then
        code=$(curl --proto '=https' --tlsv1.2 -sL -o /dev/null -w '%{http_code}' --connect-timeout 15 "$1") || return $?
        [ "$code" = 200 ] && return 0
        return 22
    else
        # shellcheck disable=SC2086
        wget -q --spider $WGET_SAFE -T 15 "$1" 2>/dev/null
        rc=$?
        [ $rc -eq 8 ] && return 22
        return $rc
    fi
}

network_fail() { # the downloader's exit code, the URL
    case "$DL:$1" in
        curl:6|curl:7|curl:28|curl:35|curl:56|wget:4)
            fail "it could not reach GitHub, where olang is downloaded from." \
                "Check that this computer is online (try opening https://github.com)," \
                "then run the install command again." \
                "${DIM}($DL exit $1 fetching $2)${RESET}" ;;
        curl:60|wget:5)
            fail "the secure connection to GitHub could not be verified." \
                "The computer's clock may be wrong, or a network filter (school, office," \
                "antivirus) may be in the way. Fix that, then run the install command again." ;;
        curl:1|curl:22)
            fail "a download was refused: $2" \
                "Wait a minute and run the install command again. If it keeps failing," \
                "please report it at $GITHUB/$REPO/issues" ;;
        *)
            fail "a download from GitHub failed: $2" \
                "Wait a minute and run the install command again. If it keeps failing," \
                "please report it at $GITHUB/$REPO/issues" \
                "${DIM}($DL exit $1)${RESET}" ;;
    esac
}

# ── versions ────────────────────────────────────────────────────────
# version_at_least 1.97 1.95 → true. Compares major.minor.
version_at_least() {
    a1=${1%%.*}; a2=${1#*.}; a2=${a2%%.*}
    b1=${2%%.*}; b2=${2#*.}; b2=${b2%%.*}
    [ "$a1" -gt "$b1" ] 2>/dev/null || { [ "$a1" -eq "$b1" ] 2>/dev/null && [ "$a2" -ge "$b2" ] 2>/dev/null; }
}

# release_at_least v1.0.0-rc.6 v1.0.0-rc.5 → true. Semantic versions:
# 1.0.0 is newer than 1.0.0-rc.9, and rc.10 newer than rc.9.
release_at_least() {
    a=${1#v}; b=${2#v}
    ac=${a%%-*}; bc=${b%%-*}
    ap=""; [ "$a" = "$ac" ] || ap=${a#*-}
    bp=""; [ "$b" = "$bc" ] || bp=${b#*-}
    for i in 1 2 3; do
        x=$(printf '%s' "$ac" | cut -d. -f$i); y=$(printf '%s' "$bc" | cut -d. -f$i)
        x=${x:-0}; y=${y:-0}
        [ "$x" -gt "$y" ] 2>/dev/null && return 0
        [ "$x" -lt "$y" ] 2>/dev/null && return 1
    done
    [ -z "$ap" ] && return 0
    [ -z "$bp" ] && return 1
    aw=${ap%%.*}; bw=${bp%%.*}
    [ "$aw" = "$bw" ] || { [ "$(printf '%s\n%s\n' "$aw" "$bw" | sort | tail -n 1)" = "$aw" ]; return; }
    an=${ap#*.}; bn=${bp#*.}
    [ "${an:-0}" -ge "${bn:-0}" ] 2>/dev/null
}

# Sets VERSION to a release tag that exists ("v1.2.3").
resolve_version() {
    if [ -n "${OLANG_VERSION:-}" ]; then
        VERSION="v${OLANG_VERSION#v}"
        case "$VERSION" in
            v[0-9]*.[0-9]*.[0-9]*) ;;
            *) fail "'$OLANG_VERSION' is not a version number." \
                   "Use the form v1.0.0 (see $GITHUB/$REPO/releases), or leave it out to get the newest." ;;
        esac
        exists "$GITHUB/$REPO/releases/tag/$VERSION"
        rc=$?
        [ $rc -eq 22 ] && fail "there is no olang release called $VERSION." \
            "See the list of releases at $GITHUB/$REPO/releases," \
            "or leave the version out to get the newest."
        [ $rc -eq 0 ] || network_fail $rc "$GITHUB/$REPO/releases/tag/$VERSION"
        VERSION_NOTE=""
        return
    fi
    VERSION_NOTE=" (the newest)"
    # The releases page's "latest" link: no API rate limit. It skips
    # prereleases, so a repository whose releases are all prereleases
    # falls through to the API list, which includes them (newest first).
    url=$(final_url "$GITHUB/$REPO/releases/latest")
    rc=$?
    [ $rc -eq 0 ] || network_fail $rc "$GITHUB/$REPO/releases/latest"
    case "$url" in
        */releases/tag/v*) VERSION="${url##*/releases/tag/}"; return ;;
    esac
    fetch "$GITHUB_API/repos/$REPO/releases?per_page=10" "$workdir/releases.json"
    rc=$?
    [ $rc -eq 22 ] && fail "GitHub would not list olang's releases just now (it limits how often one network may ask)." \
        "Wait an hour and try again, or name a release from $GITHUB/$REPO/releases:" \
        "  curl -fsSL $INSTALL_URL | sh -s -- --version v1.0.0"
    [ $rc -eq 0 ] || network_fail $rc "$GITHUB_API/repos/$REPO/releases"
    # one field per line, whether GitHub sent the JSON pretty or compact
    VERSION=$(tr ',' '\n' <"$workdir/releases.json" | sed -n 's/.*"tag_name": *"\(v[^"]*\)".*/\1/p' | head -n 1)
    [ -n "$VERSION" ] || fail "olang has no releases to install yet." \
        "You can build it from source instead: see $HELP_URL"
}

# ── which computer is this? ─────────────────────────────────────────
# Sets OS, ARCH, NICE_OS, PLATFORM (the release's name for this kind of
# computer, empty when there is none), FALLBACK (a second build to try)
# and WHY_NO_DOWNLOAD.
detect_platform() {
    OS=$(uname -s)
    ARCH=$(uname -m)
    PLATFORM=""
    FALLBACK=""
    WHY_NO_DOWNLOAD=""
    GUI_NOTE=""
    case "$OS" in
        Darwin)
            # a shell running under Rosetta reports x86_64 on Apple silicon:
            # install the native build there.
            if [ "$ARCH" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = 1 ]; then
                ARCH=arm64
            fi
            case "$ARCH" in
                arm64)  PLATFORM="macos-arm64"; NICE_OS="macOS, Apple silicon" ;;
                x86_64) PLATFORM="macos-x64";   NICE_OS="macOS, Intel" ;;
                *)      NICE_OS="macOS on $ARCH"; WHY_NO_DOWNLOAD="there is no ready-made olang for $ARCH Macs" ;;
            esac
            mac_version=$(sw_vers -productVersion 2>/dev/null)
            NICE_OS="$NICE_OS${mac_version:+, $mac_version}" ;;
        Linux)
            case "$ARCH" in
                x86_64|amd64)  a=x64 ;;
                aarch64|arm64) a=arm64 ;;
                *)             a="" ;;
            esac
            NICE_OS="Linux, $ARCH"
            grep -qi microsoft "${_OLANG_PROC_VERSION:-/proc/version}" 2>/dev/null && NICE_OS="Linux (WSL), $ARCH"
            if [ -z "$a" ]; then
                case "$ARCH" in
                    armv6*|armv7*|armhf|i386|i486|i586|i686)
                        WHY_NO_DOWNLOAD="olang's ready-made downloads are for 64-bit computers, and this is a 32-bit $ARCH system. On a Raspberry Pi, the 64-bit Raspberry Pi OS runs the ready-made olang" ;;
                    *)  WHY_NO_DOWNLOAD="there is no ready-made olang for $ARCH Linux" ;;
                esac
                return
            fi
            libc=glibc
            if have ldd; then
                ldd --version 2>&1 | grep -qi musl && libc=musl
            elif ls /lib/ld-musl-* >/dev/null 2>&1; then
                libc=musl
            fi
            if [ "$libc" = musl ]; then
                # Alpine and other musl systems: the static build
                PLATFORM="linux-$a-musl"
                NICE_OS="$NICE_OS, musl"
                GUI_NOTE="musl"
                return
            fi
            glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | sed -n 's/^glibc //p')
            [ -n "$glibc" ] || glibc=$(ldd --version 2>&1 | head -n 1 | sed -n 's/.* \([0-9][0-9]*\.[0-9][0-9]*\)$/\1/p')
            GLIBC="$glibc"
            if [ -n "$glibc" ] && ! version_at_least "$glibc" "$MIN_GLIBC"; then
                # older than the standard build: the static one runs anywhere
                PLATFORM="linux-$a-musl"
                GUI_NOTE="old-glibc"
            else
                PLATFORM="linux-$a"
                FALLBACK="linux-$a-musl"
            fi ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            fail "this is the installer for macOS and Linux, and this terminal runs on Windows." \
                "In PowerShell, install olang with:" \
                "  irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex" \
                "Or use olang inside WSL (Windows Subsystem for Linux): run  wsl --install" \
                "in PowerShell, restart, open the Ubuntu app, and run the install command there." ;;
        *)
            NICE_OS="$OS, $ARCH"
            WHY_NO_DOWNLOAD="there is no ready-made olang for $OS" ;;
    esac
}

# Stops, with what to do, when INSTALL_DIR cannot be written.
check_install_dir() {
    if ! mkdir -p "$INSTALL_DIR" 2>/dev/null || ! [ -w "$INSTALL_DIR" ]; then
        [ -z "${OLANG_INSTALL_DIR:-}" ] && fail "you do not have permission to write to $(tilde "$INSTALL_DIR")." \
            "It probably belongs to another user, for example after an earlier install run with sudo." \
            "Give it back to yourself, then run the install command again (without sudo):" \
            "  sudo chown -R \"\$(whoami)\" \"$(dirname "$INSTALL_DIR")\""
        fail "you do not have permission to write to $INSTALL_DIR." \
            "Choose a folder you own instead, for example:" \
            "  curl -fsSL $INSTALL_URL | sh -s -- --dir \"\$HOME/.olang/bin\""
    fi
}

main() {
    ACTION=install
    parse_args "$@"
    [ "${OLANG_QUIET:-}" = 1 ] && QUIET=1
    setup_output
    INSTALL_DIR="${OLANG_INSTALL_DIR:-${OLANG_HOME:-$HOME/.olang}/bin}"
    case "$INSTALL_DIR" in /*) ;; *) INSTALL_DIR="$(pwd)/$INSTALL_DIR" ;; esac

    if [ "$ACTION" = uninstall ]; then
        uninstall
        return
    fi

    # `curl ... | sudo sh` installs into root's home, out of the person's
    # reach. A root shell of its own (a container, a server) is fine.
    if [ "$(id -u 2>/dev/null)" = 0 ] && [ -n "${SUDO_USER:-}" ] && [ -z "${OLANG_INSTALL_DIR:-}" ]; then
        fail "the installer was run with sudo, which it does not need." \
            "With sudo it would install olang for the administrator instead of for you." \
            "Run it again without sudo:" \
            "  curl -fsSL $INSTALL_URL | sh"
    fi

    say ""
    say "  ${BOLD}Installing olang${RESET}"
    say ""

    pick_downloader
    missing=""
    for tool in tar gzip uname mktemp; do have "$tool" || missing="$missing $tool"; done
    if have sha256sum; then sha() { sha256sum "$1" | cut -d' ' -f1; }
    elif have shasum; then sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
    elif have openssl; then sha() { openssl dgst -sha256 -r "$1" | cut -d' ' -f1; }
    else missing="$missing sha256sum"; fi
    [ -z "$missing" ] || fail "this computer is missing tools the installer needs:$missing." \
        "On Debian or Ubuntu:  sudo apt install coreutils tar gzip" \
        "On Alpine:            apk add coreutils tar gzip" \
        "Then run the install command again."

    workdir=$(mktemp -d 2>/dev/null || mktemp -d -t olang) ||
        fail "it could not create a temporary folder." "Check that your disk is not full, then try again."
    trap 'rm -rf "$workdir"' EXIT
    trap 'exit 130' INT TERM

    detect_platform
    row "Computer" "$NICE_OS"
    check_install_dir
    resolve_version
    row "Version" "${VERSION#v}$VERSION_NOTE"
    row "Folder" "$(tilde "$INSTALL_DIR")"
    say ""

    # an install of this same version: nothing to download
    current=$("$INSTALL_DIR/olang" --version 2>/dev/null | sed -n 's/^olang \([^ ]*\).*/\1/p')
    if [ "$current" = "${VERSION#v}" ] && [ "${OLANG_FORCE:-}" != 1 ] && [ "${OLANG_FROM_SOURCE:-}" != 1 ]; then
        done_ "olang ${VERSION#v} is already installed here"
        put_on_path
        finish
        return
    fi
    PREVIOUS="$current"

    if [ -z "$PLATFORM" ] || [ "${OLANG_FROM_SOURCE:-}" = 1 ]; then
        [ -n "$WHY_NO_DOWNLOAD" ] || WHY_NO_DOWNLOAD="you asked for a source build"
        build_from_source
    else
        install_download
    fi
    put_on_path
    finish
}

install_download() {
    version_no_v="${VERSION#v}"
    base="$GITHUB/$REPO/releases/download/$VERSION"

    fetch "$base/SHA256SUMS" "$workdir/SHA256SUMS"
    rc=$?
    [ $rc -eq 22 ] && fail "release $VERSION has no SHA256SUMS file, so its download cannot be checked." \
        "Try another release from $GITHUB/$REPO/releases with --version," \
        "and please report this at $GITHUB/$REPO/issues"
    [ $rc -eq 0 ] || network_fail $rc "$base/SHA256SUMS"
    check_signature

    listed() { grep -q "  olang-$version_no_v-$1\.tar\.gz\$" "$workdir/SHA256SUMS"; }
    if ! listed "$PLATFORM"; then
        if [ -n "$FALLBACK" ] && listed "$FALLBACK"; then
            PLATFORM="$FALLBACK"; GUI_NOTE=""
        else
            if [ "$GUI_NOTE" = old-glibc ]; then
                WHY_NO_DOWNLOAD="this Linux has glibc $GLIBC, older than the $MIN_GLIBC the standard build needs, and release $VERSION has no static build"
            else
                WHY_NO_DOWNLOAD="release $VERSION has no ready-made download for $NICE_OS"
            fi
            build_from_source
            return
        fi
    fi
    download_and_unpack "$PLATFORM"

    # Run it before installing it: a Linux too old for the build fails
    # here, with a message, instead of after.
    if ! out=$("$src/olang" --version 2>&1); then
        case "$out" in
            *GLIBC*)
                if [ -n "$FALLBACK" ] && listed "$FALLBACK"; then
                    PLATFORM="$FALLBACK"; GUI_NOTE="old-glibc"
                    download_and_unpack "$PLATFORM"
                    out=$("$src/olang" --version 2>&1) || fail "the downloaded olang would not start on this computer." \
                        "Please report this at $GITHUB/$REPO/issues with this message:" "$out"
                else
                    WHY_NO_DOWNLOAD="the ready-made olang needs glibc $MIN_GLIBC or newer, and this Linux has ${GLIBC:-an older one}"
                    build_from_source
                    return
                fi ;;
            *"error while loading shared libraries"*)
                add_system_libs "$src/olang" "$out" ;;
            *"macOS"*|*"Mac OS X"*|*"dyld"*)
                fail "the downloaded olang needs a newer macOS than $(sw_vers -productVersion 2>/dev/null)." \
                    "olang runs on macOS 11 (Big Sur) or later on Apple silicon, 10.12 or later on Intel." \
                    "Update macOS in System Settings, then run the install command again." ;;
            *)
                fail "the downloaded olang would not start on this computer." \
                    "Please report this at $GITHUB/$REPO/issues with this message:" \
                    "$out" ;;
        esac
    fi
    install_bins "$src"
}

# download_and_unpack PLATFORM: fetch the tarball, check it against
# SHA256SUMS and unpack it; sets src to the unpacked folder.
download_and_unpack() {
    tarball="olang-$version_no_v-$1.tar.gz"
    live=""
    [ -z "$QUIET" ] && [ -t 2 ] && [ "$DL" = curl ] && live=1
    [ -n "$live" ] && printf '  Downloading %s\n' "$tarball" >&2
    fetch "$base/$tarball" "$workdir/$tarball" progress
    rc=$?
    [ $rc -eq 0 ] || network_fail $rc "$base/$tarball"
    # the progress bar has done its job: clear it and its heading
    [ -n "$live" ] && printf '\033[2A\033[J' >&2
    expected=$(sed -n "s/^\([0-9a-f]\{64\}\)  *$tarball\$/\1/p" "$workdir/SHA256SUMS" | head -n 1)
    actual=$(sha "$workdir/$tarball")
    { [ -n "$actual" ] && [ "$actual" = "$expected" ]; } || fail \
        "the download did not match its published checksum, so it was not installed." \
        "This is usually a download cut short. Run the install command again." \
        "${DIM}(expected $expected, got $actual)${RESET}"
    mb=$(( ($(wc -c <"$workdir/$tarball") + 524288) / 1048576 ))
    [ "$mb" -gt 0 ] && mb="$mb MB" || mb="under 1 MB"
    done_ "Downloaded olang ${VERSION#v} ($mb), checksum matches"
    rm -rf "$workdir/olang-$version_no_v-$1"
    tar -xzf "$workdir/$tarball" -C "$workdir" ||
        fail "the download could not be unpacked." "Run the install command again."
    src="$workdir/olang-$version_no_v-$1"
    [ -f "$src/olang" ] && [ -f "$src/otc" ] ||
        fail "the download did not contain olang and otc." \
            "Please report this at $GITHUB/$REPO/issues"
}

# check_signature: SHA256SUMS must be signed by olang's release key (once
# signing is set up). A bad signature always stops the install.
check_signature() {
    [ -n "$SIGNING_KEY" ] || return 0
    fetch "$GITHUB/$REPO/releases/download/$VERSION/SHA256SUMS.sig" "$workdir/SHA256SUMS.sig"
    rc=$?
    if [ $rc -eq 22 ]; then
        if [ -n "$SIGNED_FROM" ] && ! release_at_least "$VERSION" "$SIGNED_FROM"; then
            note "Release $VERSION is older than olang's release signatures (they start at $SIGNED_FROM)," \
                "so only its checksum was checked."
            return 0
        fi
        [ "${OLANG_ALLOW_UNSIGNED:-}" = 1 ] && {
            note "Release $VERSION has no signature. Going on because OLANG_ALLOW_UNSIGNED=1."
            return 0
        }
        fail "release $VERSION has no signature, so the installer cannot tell that it came from olang." \
            "Every release from $SIGNED_FROM on is signed, so this is unexpected." \
            "Please report it at $GITHUB/$REPO/issues and install another release with --version."
    fi
    [ $rc -eq 0 ] || network_fail $rc "$GITHUB/$REPO/releases/download/$VERSION/SHA256SUMS.sig"
    if ! have openssl; then
        [ "${OLANG_REQUIRE_SIGNATURE:-}" = 1 ] && fail "the release signature cannot be checked, because this computer has no 'openssl'." \
            "Install openssl (on Debian or Ubuntu: sudo apt install openssl), then run the install command again."
        note "The release signature was not checked, because this computer has no 'openssl'." \
            "The download's checksum is still checked. Install openssl to check both."
        return 0
    fi
    printf '%s\n' "$SIGNING_KEY" >"$workdir/olang-release.pub"
    if openssl dgst -sha256 -verify "$workdir/olang-release.pub" \
        -signature "$workdir/SHA256SUMS.sig" "$workdir/SHA256SUMS" >/dev/null 2>&1; then
        done_ "Signature verified: release $VERSION was signed by olang's release key"
    else
        fail "the release's signature does not match olang's signing key, so nothing was installed." \
            "The files on the download server may have been changed by someone other than olang." \
            "Please report this at $GITHUB/$REPO/issues and do not install this release by hand."
    fi
}

# add_system_libs BINARY OUTPUT: a Linux without a library the build
# links names the package that has it, offers to install it with the
# system's package manager, and goes on once the binary starts; else it
# stops with the exact command.
add_system_libs() {
    libs=$(ldd "$1" 2>/dev/null | awk '/not found/ { print $1 }')
    [ -n "$libs" ] || libs=$(printf '%s\n' "$2" | sed -n 's/.*shared libraries: \([^:]*\):.*/\1/p')
    if have apt-get; then pm=apt
    elif have dnf; then pm=dnf
    elif have yum; then pm=yum
    elif have pacman; then pm=pacman
    elif have zypper; then pm=zypper
    elif have apk; then pm=apk
    else pm=""; fi
    pkgs=""
    unknown=""
    for lib in $libs; do
        case "$pm:$lib" in
            apt:libfontconfig.so.*|zypper:libfontconfig.so.*)          pkg=libfontconfig1 ;;
            dnf:libfontconfig.so.*|yum:libfontconfig.so.*|pacman:libfontconfig.so.*|apk:libfontconfig.so.*) pkg=fontconfig ;;
            apt:libgcc_s.so.*)                        pkg=libgcc-s1 ;;
            dnf:libgcc_s.so.*|yum:libgcc_s.so.*|apk:libgcc_s.so.*) pkg=libgcc ;;
            pacman:libgcc_s.so.*)                     pkg=gcc-libs ;;
            zypper:libgcc_s.so.*)                     pkg=libgcc_s1 ;;
            *)                                        pkg=""; unknown="$unknown $lib" ;;
        esac
        [ -n "$pkg" ] && pkgs="$pkgs $pkg"
    done
    if [ -z "$pm" ] || [ -n "$unknown" ] || [ -z "$pkgs" ]; then
        fail "olang needs a system library this computer does not have:${unknown:- $libs}." \
            "Install the package that provides it with your system's package manager," \
            "then run the install command again. If you are not sure which one, report it at" \
            "$GITHUB/$REPO/issues with this message:" "$2"
    fi
    case "$pm" in
        apt)    cmd="apt-get install -y$pkgs" ;;
        dnf)    cmd="dnf install -y$pkgs" ;;
        yum)    cmd="yum install -y$pkgs" ;;
        pacman) cmd="pacman -S --needed --noconfirm$pkgs" ;;
        zypper) cmd="zypper install -y$pkgs" ;;
        apk)    cmd="apk add$pkgs" ;;
    esac
    [ "$(id -u)" = 0 ] || cmd="sudo $cmd"
    say ""
    say "  olang needs a small system package this computer does not have yet:$pkgs"
    if ask "Install it now? This runs: $cmd"; then
        # stdin from the terminal when there is one (sudo's password):
        # under `curl ... | sh` stdin is this script
        if (: </dev/tty) 2>/dev/null; then sh -c "$cmd" </dev/tty; else sh -c "$cmd" </dev/null; fi ||
            fail "installing$pkgs did not finish." \
                "Run this yourself, then run the install command again:" "  $cmd"
        out=$("$1" --version 2>&1) || fail "olang still would not start after installing$pkgs." \
            "Please report this at $GITHUB/$REPO/issues with this message:" "$out"
        done_ "Installed$pkgs"
    else
        fail "olang needs$pkgs, which is not installed." \
            "Run this, then run the install command again:" "  $cmd"
    fi
}

# install_bins DIR: copy DIR/olang and DIR/otc into INSTALL_DIR. Each is
# written beside its final name and renamed over it, which also replaces
# a symlink left by `otc update` rather than writing through it.
install_bins() {
    check_install_dir
    for bin in olang otc; do
        { cp "$1/$bin" "$INSTALL_DIR/.$bin.new" && chmod 755 "$INSTALL_DIR/.$bin.new" &&
            mv -f "$INSTALL_DIR/.$bin.new" "$INSTALL_DIR/$bin"; } ||
            fail "copying $bin into $(tilde "$INSTALL_DIR") failed, perhaps because the disk is full." \
                "Free some space and run the install command again."
        # a file downloaded by a browser carries macOS's quarantine flag;
        # curl does not set it, but clear it in case a proxy tool did.
        [ "$OS" = Darwin ] && xattr -d com.apple.quarantine "$INSTALL_DIR/$bin" 2>/dev/null
    done
    "$INSTALL_DIR/olang" --version >/dev/null 2>&1 ||
        fail "olang was copied to $(tilde "$INSTALL_DIR") but does not start from there." \
            "If that folder is on a disk mounted 'noexec', choose another with --dir."
    if [ "${PREVIOUS:-}" = "${VERSION#v}" ]; then
        done_ "Reinstalled olang ${VERSION#v}"
    elif [ -n "${PREVIOUS:-}" ]; then
        done_ "Updated olang and otc from $PREVIOUS to ${VERSION#v}"
    else
        done_ "Installed olang and otc"
    fi
    case "${GUI_NOTE:-}" in
        musl) note "This is olang's static Linux build, which runs on musl systems such as Alpine." \
                "Everything works except windows: gui.available() is false." ;;
        old-glibc) note "This Linux is older than olang's standard build supports (glibc $MIN_GLIBC)," \
                "so it got the static build, which runs anywhere but cannot open windows." ;;
    esac
}

# Builds olang with Cargo when no ready-made download fits.
build_from_source() {
    say ""
    say "  No ready-made download fits this computer: $WHY_NO_DOWNLOAD."
    say "  olang can be built from its source code instead. That needs Rust $MIN_RUST or newer,"
    say "  and takes 10 to 30 minutes and about 3 GB of disk space."
    say ""
    if ! have cargo && [ -x "$HOME/.cargo/bin/cargo" ]; then
        PATH="$HOME/.cargo/bin:$PATH"; export PATH
    fi
    if ! have cargo; then
        say "  Rust is not installed. The official Rust installer (https://rustup.rs) can set it up;"
        say "  it adds about 1 GB under ~/.rustup and ~/.cargo."
        if ask "Install Rust now, then build olang?"; then
            say ""
            say "  Installing Rust"
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal ||
                fail "the Rust installer failed." "Install Rust by hand from https://rustup.rs, then run this again."
            PATH="$HOME/.cargo/bin:$PATH"; export PATH
        else
            fail "a source build needs Rust, and Rust is not installed." \
                "Install it with:" \
                "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" \
                "then open a new terminal and run the olang install command again."
        fi
    fi
    rust=$(rustc --version 2>/dev/null | sed -n 's/^rustc \([0-9]*\.[0-9]*\).*/\1/p')
    if [ -n "$rust" ] && ! version_at_least "$rust" "$MIN_RUST"; then
        if have rustup; then
            say "  This computer has Rust $rust, and olang needs $MIN_RUST or newer."
            if ask "Update Rust now (rustup update stable)?"; then
                rustup update stable || fail "updating Rust failed." "Run  rustup update stable  and then try again."
            else
                fail "olang needs Rust $MIN_RUST or newer, and this computer has $rust." \
                    "Update it with:  rustup update stable" "then run the install command again."
            fi
        else
            fail "olang needs Rust $MIN_RUST or newer, and this computer has $rust, which rustup does not manage." \
                "Update Rust with the tool that installed it (for example: brew upgrade rust)," \
                "or install rustup from https://rustup.rs, then run the install command again."
        fi
    fi
    if [ "$OS" = Linux ]; then
        say "  On Linux the build also needs a C compiler. On Debian or Ubuntu:"
        say "    sudo apt install build-essential"
        say ""
    fi
    ask "Build olang $VERSION from source now?" || fail "the source build was not started." \
        "To build it yourself, run:" \
        "  cargo install --locked --git $GITHUB/$REPO.git --tag $VERSION olang otc"
    say ""
    say "  Building olang $VERSION from source. This takes a while."
    cargo install --locked --git "$GITHUB/$REPO.git" --tag "$VERSION" --root "$workdir/root" olang otc ||
        fail "the source build failed (the messages above say where)." \
            "The installation guide lists what a build needs: $HELP_URL" \
            "If you are stuck, report it at $GITHUB/$REPO/issues with those messages."
    install_bins "$workdir/root/bin"
}

# Which startup file the person's login shell reads, and the line that
# puts INSTALL_DIR on PATH there. Sets RC, LINE and DIR_TEXT.
shell_rc() {
    # as written into a startup file: $HOME kept symbolic when it fits
    case "$INSTALL_DIR" in
        "$HOME"/*) DIR_TEXT="\$HOME${INSTALL_DIR#"$HOME"}" ;;
        *)         DIR_TEXT="$INSTALL_DIR" ;;
    esac
    shell_name=$(basename "${SHELL:-sh}")
    LINE="export PATH=\"$DIR_TEXT:\$PATH\""
    case "$shell_name" in
        zsh)  RC="${ZDOTDIR:-$HOME}/.zshrc" ;;
        bash) # macOS terminals start login shells (~/.bash_profile), but a
              # new ~/.bash_profile would hide an existing ~/.profile
              if [ "$(uname -s)" != Darwin ]; then RC="$HOME/.bashrc"
              elif [ -f "$HOME/.bash_profile" ]; then RC="$HOME/.bash_profile"
              elif [ -f "$HOME/.bash_login" ]; then RC="$HOME/.bash_login"
              elif [ -f "$HOME/.profile" ]; then RC="$HOME/.profile"
              else RC="$HOME/.bash_profile"; fi ;;
        fish) RC="$HOME/.config/fish/conf.d/olang.fish"; LINE="fish_add_path \"$DIR_TEXT\"" ;;
        *)    RC="$HOME/.profile" ;;
    esac
}

MARK="# olang (added by the olang installer)"

# Puts INSTALL_DIR on PATH in the startup file of the person's login
# shell (once: a second run finds its line and adds nothing).
put_on_path() {
    PATH_CHANGED=""
    PATH_HINT=""
    NEEDS_NEW_SHELL=""
    shell_rc
    case ":$PATH:" in *":$INSTALL_DIR:"*) return ;; esac
    NEEDS_NEW_SHELL=1
    if [ "${OLANG_NO_MODIFY_PATH:-}" = 1 ]; then
        PATH_HINT="To use olang in new terminals, add this line to $(tilde "$RC"):"
        return
    fi
    if [ -f "$RC" ] && { grep -qF "$DIR_TEXT" "$RC" || grep -qF "$INSTALL_DIR" "$RC"; } 2>/dev/null; then
        PATH_CHANGED="$(tilde "$RC") already adds $(tilde "$INSTALL_DIR") to your PATH"
        return
    fi
    mkdir -p "$(dirname "$RC")" 2>/dev/null
    if printf '\n%s\n%s\n' "$MARK" "$LINE" >>"$RC" 2>/dev/null; then
        PATH_CHANGED="Added $(tilde "$INSTALL_DIR") to your PATH in $(tilde "$RC")"
    else
        PATH_HINT="Could not write to $(tilde "$RC"). To use olang in new terminals, add this line to it:"
    fi
}

finish() {
    [ -n "${PATH_CHANGED:-}" ] && done_ "$PATH_CHANGED"
    if [ -n "${PATH_HINT:-}" ]; then
        say ""
        say "  $PATH_HINT"
        say "    $LINE"
    fi
    # another olang earlier on PATH (an old `cargo install`, say) would
    # answer `olang` instead of this one. (A line just added to a startup
    # file goes last there and puts this install first.)
    other=$(command -v olang 2>/dev/null)
    if [ -z "${NEEDS_NEW_SHELL:-}" ] && [ -n "$other" ] && [ "$other" != "$INSTALL_DIR/olang" ]; then
        say ""
        note "Another olang, at $(tilde "$other"), comes first on your PATH, so typing 'olang' runs that one." \
            "Remove it to use this install. For one installed by Cargo: cargo uninstall olang otc"
    fi
    say ""
    if [ -n "${NEEDS_NEW_SHELL:-}" ]; then
        # a terminal reads its startup file once, when it opens: this one
        # has not seen the new PATH yet, so say how to bring it in
        say "  ${BOLD}olang is installed.${RESET} This terminal doesn't know where it is yet, so first either:"
        say ""
        say "    - quit and reopen your terminal, or"
        if [ -n "${PATH_CHANGED:-}" ]; then
            say "    - run this here:  ${BOLD}source $(tilde "$RC")${RESET}"
        else
            say "    - run this here:  ${BOLD}$LINE${RESET}"
        fi
        say ""
        say "  Then check it works:"
        say ""
        say "    ${BOLD}olang --version${RESET}"
    else
        say "  ${BOLD}olang is ready.${RESET} Run:"
        say ""
        say "    ${BOLD}olang --version${RESET}"
    fi
    say ""
    say "  Start with the tour at https://olang.dev/book/tour/"
    say "  ${DIM}To remove olang later: curl -fsSL $INSTALL_URL | sh -s -- --uninstall${RESET}"
    say ""
}

# ── uninstall ───────────────────────────────────────────────────────
# Removes olang and otc from INSTALL_DIR and the PATH line this script
# added. Packages, history and toolchains in ~/.olang stay unless the
# person says to delete them.
uninstall() {
    say ""
    say "  ${BOLD}Removing olang${RESET}"
    say ""
    removed=""
    for bin in olang otc; do
        if [ -e "$INSTALL_DIR/$bin" ] || [ -L "$INSTALL_DIR/$bin" ]; then
            rm -f "$INSTALL_DIR/$bin" || fail "it could not remove $(tilde "$INSTALL_DIR/$bin")." \
                "Check that you own that folder, then run the uninstall again."
            removed="$removed $bin"
        fi
    done
    if [ -n "$removed" ]; then
        case "$removed" in " olang otc") removed=" olang and otc" ;; esac
        done_ "Removed$removed from $(tilde "$INSTALL_DIR")"
    else
        done_ "olang was not installed in $(tilde "$INSTALL_DIR")"
    fi
    rmdir "$INSTALL_DIR" 2>/dev/null

    # the startup-file line: the marker comment and the line after it
    # (fish has a file of its own, removed whole)
    for rc in "${ZDOTDIR:-$HOME}/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
        if ! { [ -f "$rc" ] && grep -qF "$MARK" "$rc"; } 2>/dev/null; then continue; fi
        # written back with cat, so the file keeps its owner, mode and links
        if awk -v mark="$MARK" '
            skip { skip = 0; next }
            $0 == mark { skip = 1; blank = 0; next }
            { if (blank) print ""; blank = ($0 == ""); if (!blank) print }
            END { if (blank) print "" }
        ' "$rc" >"$rc.olang-tmp" && cat "$rc.olang-tmp" >"$rc"; then
            done_ "Removed the PATH line from $(tilde "$rc")"
        else
            note "Could not edit $(tilde "$rc"). Remove the two lines after '$MARK' yourself."
        fi
        rm -f "$rc.olang-tmp"
    done
    fish_rc="$HOME/.config/fish/conf.d/olang.fish"
    if [ -f "$fish_rc" ] && grep -qF "$MARK" "$fish_rc" 2>/dev/null; then
        rm -f "$fish_rc" && done_ "Removed $(tilde "$fish_rc")"
    fi

    data="${OLANG_HOME:-$HOME/.olang}"
    rmdir "$data" 2>/dev/null
    if [ -d "$data" ] && [ -n "$(ls -A "$data" 2>/dev/null)" ]; then
        if ask "Also delete $(tilde "$data"), which holds your packages, REPL history and toolchains?" &&
            [ "${OLANG_YES:-}" != 1 ]; then
            rm -rf "$data" && done_ "Deleted $(tilde "$data")"
        else
            say ""
            say "  Your packages, REPL history and toolchains in $(tilde "$data") were kept."
            say "  To delete them too:  rm -rf $(tilde "$data")"
        fi
    fi
    say ""
    say "  ${BOLD}olang is removed.${RESET} Open a new terminal window to finish."
    say ""
}

main "$@"
