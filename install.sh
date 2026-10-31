#!/bin/sh
# The olang installer.
#
#   curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh
#
# Downloads the ready-made olang for this computer from the newest GitHub
# release, checks it against the release's SHA256SUMS, installs `olang`
# and `otc` into ~/.olang/bin, and puts that folder on your PATH. Where no
# ready-made download fits, it explains why and offers to build olang
# from source instead (which needs Rust).
#
# Settings (all optional), given as `... | OLANG_VERSION=v1.0.0 sh`:
#   OLANG_VERSION         a release to install, e.g. v1.0.0 (default: newest)
#   OLANG_INSTALL_DIR     where the programs go (default: ~/.olang/bin)
#   OLANG_NO_MODIFY_PATH  set to 1 to leave your shell startup files alone
#   OLANG_FROM_SOURCE     set to 1 to build from source even when a
#                         ready-made download exists
#   OLANG_YES             set to 1 to answer "yes" to every question
#
# POSIX sh; safe to run again (it replaces the install in place). The
# whole script is a function called on the last line, so a download cut
# short part-way runs nothing.
set -u

REPO="ooyeku/olang"
GITHUB="${OLANG_GITHUB:-https://github.com}"
GITHUB_API="${OLANG_GITHUB_API:-https://api.github.com}"
# The oldest Rust that builds olang: keep equal to `rust-version` in
# Cargo.toml (tests/install_script_test.rs checks).
MIN_RUST="1.97"

# ── output ──────────────────────────────────────────────────────────
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    BOLD=$(printf '\033[1m'); DIM=$(printf '\033[2m'); RED=$(printf '\033[31m')
    GREEN=$(printf '\033[32m'); RESET=$(printf '\033[0m')
else
    BOLD=""; DIM=""; RED=""; GREEN=""; RESET=""
fi
say()  { printf '%s\n' "$*"; }
step() { printf '%s==>%s %s\n' "$BOLD" "$RESET" "$*"; }
ok()   { printf '%s  ok%s  %s\n' "$GREEN" "$RESET" "$*"; }
# fail "what went wrong" "what to do" ["more"...]: the problem in plain
# words, then the next step, then stop.
fail() {
    printf '\n%sThe olang installer stopped:%s %s\n' "$RED$BOLD" "$RESET" "$1" >&2
    shift
    for line in "$@"; do printf '  %s\n' "$line" >&2; done
    printf '\n%sHelp: https://olang.dev/book/installation/%s\n' "$DIM" "$RESET" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }
# a path as people read it: ~/.olang/bin rather than /Users/you/.olang/bin
tilde() { case "$1" in "$HOME"/*) printf '~%s' "${1#"$HOME"}" ;; *) printf '%s' "$1" ;; esac; }

# ask "question": yes when OLANG_YES=1, no when there is nobody to ask
# (no terminal), else whatever the person types. Reads the terminal, not
# stdin, because under `curl ... | sh` stdin is this script.
ask() {
    [ "${OLANG_YES:-}" = 1 ] && return 0
    { [ -r /dev/tty ] && [ -w /dev/tty ]; } 2>/dev/null || return 1
    printf '%s [y/N] ' "$1" 2>/dev/null >/dev/tty || return 1
    read -r answer </dev/tty 2>/dev/null || return 1
    case "$answer" in y|Y|yes|YES|Yes) return 0 ;; *) return 1 ;; esac
}

# fetch URL FILE [progress]: download, with a plain message per failure.
fetch() {
    if [ "${3:-}" = progress ] && [ -t 2 ]; then
        curl -fL --progress-bar --retry 2 --connect-timeout 15 -o "$2" "$1"
    else
        curl -fsSL --retry 2 --connect-timeout 15 -o "$2" "$1"
    fi
}
network_fail() { # curl's exit code, the URL
    case "$1" in
        6|7|28|35|56)
            fail "it could not reach GitHub, where olang is downloaded from." \
                "Check that this computer is online (open https://github.com in a browser)," \
                "then run the install command again." \
                "${DIM}(curl exit $1 fetching $2)${RESET}" ;;
        60)
            fail "the secure connection to GitHub could not be verified." \
                "This usually means the computer's clock is wrong, or a network filter" \
                "(school, office, antivirus) is in the way. Fix that and run the command again." ;;
        *)
            fail "a download from GitHub failed: $2" \
                "Wait a minute and run the install command again. If it keeps failing," \
                "please report it at $GITHUB/$REPO/issues" \
                "${DIM}(curl exit $1)${RESET}" ;;
    esac
}

main() {
    INSTALL_DIR="${OLANG_INSTALL_DIR:-${OLANG_HOME:-$HOME/.olang}/bin}"

    say ""
    say "${BOLD}Installing olang${RESET}"
    say ""

    # ── tools this script needs ─────────────────────────────────────
    have curl || fail "this computer has no 'curl', which the installer uses to download olang." \
        "On Debian or Ubuntu: sudo apt install curl   On Fedora: sudo dnf install curl" \
        "then run the install command again."
    missing=""
    for tool in tar gzip uname mktemp; do have "$tool" || missing="$missing $tool"; done
    if have shasum; then sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
    elif have sha256sum; then sha() { sha256sum "$1" | cut -d' ' -f1; }
    else missing="$missing shasum"; fi
    [ -z "$missing" ] || fail "this computer is missing tools the installer needs:$missing" \
        "On Debian or Ubuntu: sudo apt install coreutils tar gzip" \
        "then run the install command again."

    # ── which computer is this? ─────────────────────────────────────
    os=$(uname -s)
    arch=$(uname -m)
    platform=""
    why_no_download=""
    case "$os" in
        Darwin)
            # a shell running under Rosetta reports x86_64 on Apple silicon:
            # install the native build there.
            if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = 1 ]; then
                arch=arm64
            fi
            case "$arch" in
                arm64)  platform="macos-arm64"; nice_os="macOS (Apple silicon)" ;;
                x86_64) platform="macos-x64";   nice_os="macOS (Intel)" ;;
                *)      nice_os="macOS on $arch"; why_no_download="there is no ready-made olang for $arch Macs" ;;
            esac
            macos_major=$(sw_vers -productVersion 2>/dev/null | cut -d. -f1)
            if [ -n "$platform" ] && [ -n "$macos_major" ] && [ "$macos_major" -lt 11 ] 2>/dev/null; then
                say "Note: this Mac runs macOS $(sw_vers -productVersion). olang is tested on macOS 11 (Big Sur) and later;"
                say "      it may still work here."
            fi ;;
        Linux)
            case "$arch" in
                x86_64)        platform="linux-x64" ;;
                aarch64|arm64) platform="linux-arm64" ;;
            esac
            nice_os="Linux ($arch)"
            [ -n "$platform" ] || why_no_download="there is no ready-made olang for $arch Linux"
            if [ -n "$platform" ] && ldd --version 2>&1 | grep -qi musl; then
                # Alpine and other musl systems cannot run the glibc builds
                platform=""
                why_no_download="the ready-made Linux olang needs glibc, and this system uses a different C library (Alpine?)"
            fi ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            fail "olang has no ready-made download for Windows yet." \
                "Use olang inside WSL (Windows Subsystem for Linux):" \
                "  1. Open PowerShell and run:  wsl --install" \
                "  2. Restart the computer, then open the new \"Ubuntu\" app." \
                "  3. In Ubuntu, run this install command again:" \
                "     curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | sh" ;;
        *)
            nice_os="$os ($arch)"
            why_no_download="there is no ready-made olang for $os" ;;
    esac
    [ -n "$platform" ] && say "This computer: $nice_os"

    check_install_dir

    # ── which release? ──────────────────────────────────────────────
    resolve_version
    say "olang version: ${VERSION#v}"

    if [ -z "$platform" ] || [ "${OLANG_FROM_SOURCE:-}" = 1 ]; then
        [ -n "$why_no_download" ] || why_no_download="OLANG_FROM_SOURCE=1 asked for a source build"
        build_from_source
    else
        install_download
    fi
    put_on_path
    finish
}

# Sets VERSION to a release tag that exists ("v1.2.3").
resolve_version() {
    if [ -n "${OLANG_VERSION:-}" ]; then
        VERSION="v${OLANG_VERSION#v}"
        case "$VERSION" in
            v[0-9]*.[0-9]*.[0-9]*) ;;
            *) fail "OLANG_VERSION='$OLANG_VERSION' is not a version number." \
                   "Use the form v1.0.0 (see $GITHUB/$REPO/releases), or leave it out to get the newest." ;;
        esac
        code=$(curl -sL -o /dev/null -w '%{http_code}' --connect-timeout 15 "$GITHUB/$REPO/releases/tag/$VERSION")
        rc=$?
        [ $rc -eq 0 ] || network_fail $rc "$GITHUB/$REPO/releases/tag/$VERSION"
        [ "$code" = 200 ] || fail "there is no olang release called $VERSION." \
            "See the list of releases at $GITHUB/$REPO/releases," \
            "or leave OLANG_VERSION out to get the newest."
        return
    fi
    step "Finding the newest olang release"
    # The releases page's "latest" link: no API rate limit. It skips
    # prereleases, so a repository whose releases are all prereleases
    # falls through to the API list, which includes them (newest first).
    url=$(curl -sL -o /dev/null -w '%{url_effective}' --connect-timeout 15 "$GITHUB/$REPO/releases/latest")
    rc=$?
    [ $rc -eq 0 ] || network_fail $rc "$GITHUB/$REPO/releases/latest"
    case "$url" in
        */releases/tag/*) VERSION="${url##*/releases/tag/}"; return ;;
    esac
    list=$(curl -fsL --connect-timeout 15 "$GITHUB_API/repos/$REPO/releases?per_page=10")
    rc=$?
    if [ $rc -eq 22 ]; then
        fail "GitHub would not list olang's releases right now (it limits how often one network may ask)." \
            "Wait an hour and try again, or name a release from $GITHUB/$REPO/releases, e.g.:" \
            "  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | OLANG_VERSION=v1.0.0 sh"
    fi
    [ $rc -eq 0 ] || network_fail $rc "$GITHUB_API/repos/$REPO/releases"
    VERSION=$(printf '%s\n' "$list" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$VERSION" ] || fail "olang has no releases to install yet." \
        "Build from source instead: see https://olang.dev/book/installation/"
}

install_download() {
    version_no_v="${VERSION#v}"
    base="$GITHUB/$REPO/releases/download/$VERSION"
    tarball="olang-$version_no_v-$platform.tar.gz"
    workdir=$(mktemp -d 2>/dev/null || mktemp -d -t olang)
    trap 'rm -rf "$workdir"' EXIT
    trap 'exit 130' INT TERM

    step "Downloading olang (about 60 MB)"
    fetch "$base/SHA256SUMS" "$workdir/SHA256SUMS" || {
        rc=$?
        [ $rc -eq 22 ] && fail "release $VERSION has no SHA256SUMS file, so its download cannot be checked." \
            "Try an earlier release from $GITHUB/$REPO/releases with OLANG_VERSION=v..." \
            "and please report this at $GITHUB/$REPO/issues"
        network_fail $rc "$base/SHA256SUMS"
    }
    if ! grep -q "  $tarball\$" "$workdir/SHA256SUMS"; then
        why_no_download="release $VERSION has no ready-made download for $nice_os"
        build_from_source
        return
    fi
    fetch "$base/$tarball" "$workdir/$tarball" progress || network_fail $? "$base/$tarball"

    step "Checking the download"
    expected=$(sed -n "s/^\([0-9a-f]\{64\}\)  *$tarball\$/\1/p" "$workdir/SHA256SUMS")
    actual=$(sha "$workdir/$tarball")
    [ -n "$actual" ] && [ "$actual" = "$expected" ] || fail \
        "the download did not match its published checksum, so it was not installed." \
        "This is usually a download cut short. Run the install command again." \
        "${DIM}(expected $expected, got $actual)${RESET}"
    ok "checksum matches SHA256SUMS"
    tar -xzf "$workdir/$tarball" -C "$workdir" ||
        fail "the download could not be unpacked." "Run the install command again."
    src="$workdir/olang-$version_no_v-$platform"

    # Run it before installing it: a Linux too old for the build fails
    # here, with a message, instead of after.
    if ! out=$("$src/olang" --version 2>&1); then
        case "$out" in
            *GLIBC*)
                glibc=$(ldd --version 2>&1 | head -n 1 | sed 's/.* //')
                why_no_download="the ready-made olang needs a newer Linux (glibc 2.39, as in Ubuntu 24.04 or Debian 13); this one has glibc $glibc"
                build_from_source
                return ;;
            *"macOS"*|*"Mac OS X"*|*"dyld"*)
                fail "the downloaded olang needs a newer macOS than $(sw_vers -productVersion 2>/dev/null)." \
                    "olang runs on macOS 11 (Big Sur) or later on Apple silicon, 10.12 or later on Intel." \
                    "Update macOS from System Settings, then run the install command again." ;;
            *)
                fail "the downloaded olang would not start on this computer." \
                    "Please report this at $GITHUB/$REPO/issues with this message:" \
                    "$out" ;;
        esac
    fi
    install_bins "$src"
}

# Stops, with what to do, when INSTALL_DIR cannot be written.
check_install_dir() {
    if ! mkdir -p "$INSTALL_DIR" 2>/dev/null || ! [ -w "$INSTALL_DIR" ]; then
        [ -z "${OLANG_INSTALL_DIR:-}" ] && fail "you do not have permission to write to $INSTALL_DIR." \
            "It probably belongs to another user, for example after an earlier install run with sudo." \
            "Give it back to yourself, then run the install command again (without sudo):" \
            "  sudo chown -R \"\$(whoami)\" \"$(dirname "$INSTALL_DIR")\""
        fail "you do not have permission to write to $INSTALL_DIR." \
            "Install somewhere you own instead, for example:" \
            "  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | OLANG_INSTALL_DIR=\"\$HOME/.olang/bin\" sh"
    fi
}

# install_bins DIR: copy DIR/olang and DIR/otc into INSTALL_DIR. Each is
# written beside its final name and renamed over it, which also replaces
# a symlink left by `otc update` rather than writing through it.
install_bins() {
    step "Installing into $(tilde "$INSTALL_DIR")"
    check_install_dir
    for bin in olang otc; do
        cp "$1/$bin" "$INSTALL_DIR/.$bin.new" && chmod 755 "$INSTALL_DIR/.$bin.new" &&
            mv -f "$INSTALL_DIR/.$bin.new" "$INSTALL_DIR/$bin" ||
            fail "copying $bin into $INSTALL_DIR failed (is the disk full?)." \
                "Free some space and run the install command again."
        # a file downloaded by a browser carries macOS's quarantine flag;
        # curl does not set it, but clear it in case a proxy tool did.
        [ "$os" = Darwin ] && xattr -d com.apple.quarantine "$INSTALL_DIR/$bin" 2>/dev/null
    done
    "$INSTALL_DIR/olang" --version >/dev/null 2>&1 ||
        fail "olang was copied to $INSTALL_DIR but does not start from there." \
            "If that folder is on a disk mounted 'noexec', choose another with OLANG_INSTALL_DIR."
    ok "installed olang and otc"
}

# Builds olang with Cargo when no ready-made download fits.
build_from_source() {
    say ""
    say "No ready-made download fits this computer: $why_no_download."
    say "olang can be built from its source code instead. That needs Rust $MIN_RUST or newer,"
    say "and takes roughly 10-30 minutes and about 3 GB of disk space."
    say ""
    if ! have cargo && [ -x "$HOME/.cargo/bin/cargo" ]; then
        PATH="$HOME/.cargo/bin:$PATH"; export PATH
    fi
    if ! have cargo; then
        say "Rust is not installed. The official Rust installer (https://rustup.rs) can set it up;"
        say "it adds about 1 GB under ~/.rustup and ~/.cargo."
        if ask "Install Rust now, then build olang?"; then
            step "Installing Rust"
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
            say "This computer has Rust $rust; olang needs $MIN_RUST or newer."
            if ask "Update Rust now (rustup update stable)?"; then
                rustup update stable || fail "updating Rust failed." "Run: rustup update stable   then try again."
            else
                fail "olang needs Rust $MIN_RUST or newer; this computer has $rust." \
                    "Update it with:  rustup update stable" "then run the install command again."
            fi
        else
            fail "olang needs Rust $MIN_RUST or newer; this computer has $rust (not managed by rustup)." \
                "Update Rust with the tool that installed it (e.g. brew upgrade rust)," \
                "or install rustup from https://rustup.rs, then run the install command again."
        fi
    fi
    if [ "$os" = Linux ]; then
        say "On Linux the build also needs a few system libraries. On Debian or Ubuntu:"
        say "  sudo apt install build-essential pkg-config libssl-dev libfontconfig1-dev libxkbcommon-dev libwayland-dev"
        say ""
    fi
    ask "Build olang $VERSION from source now?" || fail "the source build was not started." \
        "To build it yourself, run:" \
        "  cargo install --locked --git $GITHUB/$REPO.git --tag $VERSION olang otc"
    workdir=${workdir:-$(mktemp -d 2>/dev/null || mktemp -d -t olang)}
    trap 'rm -rf "$workdir"' EXIT
    trap 'exit 130' INT TERM
    step "Building olang $VERSION from source (this takes a while)"
    cargo install --locked --git "$GITHUB/$REPO.git" --tag "$VERSION" --root "$workdir/root" olang otc ||
        fail "the source build failed (the messages above say where)." \
            "The installation guide lists what a build needs: https://olang.dev/book/installation/" \
            "If you are stuck, report it at $GITHUB/$REPO/issues with those messages."
    install_bins "$workdir/root/bin"
}

# version_at_least 1.97 1.95 → true. Compares major.minor.
version_at_least() {
    a1=${1%%.*}; a2=${1#*.}; a2=${a2%%.*}
    b1=${2%%.*}; b2=${2#*.}; b2=${b2%%.*}
    [ "$a1" -gt "$b1" ] || { [ "$a1" -eq "$b1" ] && [ "$a2" -ge "$b2" ]; }
}

# Puts INSTALL_DIR on PATH in the startup file of the person's login
# shell (once: a second run finds its line and adds nothing).
put_on_path() {
    PATH_CHANGED=""
    NEEDS_NEW_SHELL=""
    case ":$PATH:" in *":$INSTALL_DIR:"*) return ;; esac
    NEEDS_NEW_SHELL=1
    # as written into a startup file: $HOME kept symbolic when it fits
    case "$INSTALL_DIR" in
        "$HOME"/*) dir_text="\$HOME${INSTALL_DIR#"$HOME"}" ;;
        *)         dir_text="$INSTALL_DIR" ;;
    esac
    shell_name=$(basename "${SHELL:-sh}")
    case "$shell_name" in
        zsh)  rc="${ZDOTDIR:-$HOME}/.zshrc"; line="export PATH=\"$dir_text:\$PATH\"" ;;
        bash) # macOS terminals start login shells (~/.bash_profile), but a
              # new ~/.bash_profile would hide an existing ~/.profile
              if [ "$os" != Darwin ]; then rc="$HOME/.bashrc"
              elif [ -f "$HOME/.bash_profile" ]; then rc="$HOME/.bash_profile"
              elif [ -f "$HOME/.bash_login" ]; then rc="$HOME/.bash_login"
              elif [ -f "$HOME/.profile" ]; then rc="$HOME/.profile"
              else rc="$HOME/.bash_profile"; fi
              line="export PATH=\"$dir_text:\$PATH\"" ;;
        fish) rc="$HOME/.config/fish/conf.d/olang.fish"; line="fish_add_path \"$dir_text\"" ;;
        *)    rc="$HOME/.profile"; line="export PATH=\"$dir_text:\$PATH\"" ;;
    esac
    if [ "$shell_name" = fish ]; then NOW_LINE="fish_add_path \"$dir_text\""
    else NOW_LINE="export PATH=\"$dir_text:\$PATH\""; fi
    if [ "${OLANG_NO_MODIFY_PATH:-}" = 1 ]; then
        PATH_HINT="Add this line to $(tilde "$rc") so new terminals find olang:"
        return
    fi
    if [ -f "$rc" ] && { grep -qF "$dir_text" "$rc" || grep -qF "$INSTALL_DIR" "$rc"; } 2>/dev/null; then
        PATH_CHANGED="$(tilde "$rc") already puts $(tilde "$INSTALL_DIR") on your PATH"
        return
    fi
    mkdir -p "$(dirname "$rc")" 2>/dev/null
    if printf '\n# olang (added by the olang installer)\n%s\n' "$line" >>"$rc" 2>/dev/null; then
        PATH_CHANGED="added $(tilde "$INSTALL_DIR") to your PATH in $(tilde "$rc")"
    else
        PATH_HINT="Could not write to $(tilde "$rc"). Add this line to it yourself:"
    fi
}

finish() {
    [ -n "${PATH_CHANGED:-}" ] && ok "$PATH_CHANGED"
    if [ -n "${PATH_HINT:-}" ]; then
        say ""
        say "$PATH_HINT"
        say "  $NOW_LINE"
    fi
    # another olang earlier on PATH (an old `cargo install`, say) would
    # answer `olang` instead of this one. (A line just added to a startup
    # file goes last there and puts this install first.)
    other=$(command -v olang 2>/dev/null)
    if [ -z "${NEEDS_NEW_SHELL:-}" ] && [ -n "$other" ] && [ "$other" != "$INSTALL_DIR/olang" ]; then
        say ""
        say "Note: another olang at $other comes first on your PATH, so typing 'olang' runs that one."
        say "      Remove it (for one installed by Cargo: cargo uninstall olang otc) to use this install."
    fi
    say ""
    if [ -n "${NEEDS_NEW_SHELL:-}" ]; then
        say "${GREEN}${BOLD}olang is installed.${RESET} Open a new terminal window, then try: ${BOLD}olang --version${RESET}"
        say "${DIM}(or paste this line here to use it in this window: $NOW_LINE)${RESET}"
    else
        say "${GREEN}${BOLD}olang is installed.${RESET} Try: ${BOLD}olang --version${RESET}"
    fi
    say "Learn more at https://olang.dev"
}

main "$@"
