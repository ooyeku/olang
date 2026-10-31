#!/bin/sh
# Runs install.sh against pretend computers and a pretend GitHub.
#
#   tests/install/harness.sh [SHELL]     (SHELL: sh, dash or bash; default sh)
#
# Nothing here touches the network or your real home folder. Each case
# gets a fresh HOME under a temporary folder, a PATH of stubs (uname,
# ldd, getconf, sw_vers, sysctl, id, curl, wget, apt-get, sudo, cargo,
# rustc) in front of a short list of real tools, and a local folder that
# stands in for github.com: the stub curl and wget serve files from it
# and refuse any call that is not HTTPS-only. tests/install_script_test.rs
# runs this under each shell it finds.
# shellcheck disable=SC2015,SC2016,SC2086,SC2088 # test code: a && ok || bad, literal $HOME and ~ in expected text
set -u

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SHELL_NAME=${1:-sh}
SHELL_BIN=$(command -v "$SHELL_NAME") || { echo "no $SHELL_NAME on this computer" >&2; exit 2; }
T=$(mktemp -d 2>/dev/null || mktemp -d -t olang-install-test)
trap 'rm -rf "$T"' EXIT
PASS=0
FAILED=0

# ── the pretend GitHub ─────────────────────────────────────────────
SRV="$T/server"
REL="$SRV/github.com/ooyeku/olang/releases"
mkdir -p "$REL/tag" "$REL/download" "$SRV/api.github.com/repos/ooyeku/olang"

if command -v sha256sum >/dev/null 2>&1; then sha() { sha256sum "$1" | cut -d' ' -f1; }
else sha() { shasum -a 256 "$1" | cut -d' ' -f1; }; fi

# fake_olang VERSION PLATFORM NAME: the stand-in program inside a
# tarball. It fails as the pretend computer says it should.
fake_olang() {
    cat <<EOF
#!/bin/sh
if [ "\${FAKE_FAIL_PLATFORM:-}" = "$2" ]; then
    echo "./$3: /lib/x86_64-linux-gnu/libc.so.6: version \\\`GLIBC_2.39' not found (required by ./$3)" >&2
    exit 1
fi
if [ -n "\${FAKE_NEED_LIB:-}" ] && [ ! -f "\$FAKE_STATE/lib-installed" ]; then
    echo "./$3: error while loading shared libraries: \$FAKE_NEED_LIB: cannot open shared object file: No such file or directory" >&2
    exit 127
fi
echo "$3 $1"
EOF
}

# release VERSION PLATFORM...: a release with a tarball per platform and
# its SHA256SUMS
release() {
    v=$1; shift
    d="$REL/download/v$v"
    mkdir -p "$d" "$T/build"
    echo "<html>v$v</html>" >"$REL/tag/v$v"
    : >"$d/SHA256SUMS"
    for p in "$@"; do
        stage="$T/build/olang-$v-$p"
        mkdir -p "$stage"
        fake_olang "$v" "$p" olang >"$stage/olang"
        fake_olang "$v" "$p" otc >"$stage/otc"
        chmod 755 "$stage/olang" "$stage/otc"
        (cd "$T/build" && tar -czf "$d/olang-$v-$p.tar.gz" "olang-$v-$p")
        printf '%s  %s\n' "$(sha "$d/olang-$v-$p.tar.gz")" "olang-$v-$p.tar.gz" >>"$d/SHA256SUMS"
    done
}

ALL="macos-arm64 macos-x64 linux-x64 linux-arm64 linux-x64-musl linux-arm64-musl"
release 1.0.0 $ALL                  # older than signing
release 1.1.0 $ALL                  # the newest, signed
release 1.2.0 $ALL                  # signed with the wrong key
release 1.3.0 $ALL                  # signing on, but no signature
release 1.4.0 $ALL                  # a checksum that does not match
release 0.9.0 macos-arm64 macos-x64 linux-x64   # before the static builds
release 0.8.0                                    # no downloads at all
rm "$REL/download/v0.8.0/SHA256SUMS"
release 1.5.0 linux-x64 linux-arm64 macos-arm64  # gnu only, no static build
echo "https://github.com/ooyeku/olang/releases/tag/v1.1.0" >"$REL/latest.redirect"
sed 's/^[0-9a-f]*  \(olang-1.4.0-macos-arm64\)/0000000000000000000000000000000000000000000000000000000000000000  \1/' \
    "$REL/download/v1.4.0/SHA256SUMS" >"$T/sums" && mv "$T/sums" "$REL/download/v1.4.0/SHA256SUMS"
# the API's list of releases (used when "latest" has no tag to show)
printf '[{"tag_name": "v1.1.0", "prerelease": true}, {"tag_name": "v1.0.0"}]\n' \
    >"$SRV/api.github.com/repos/ooyeku/olang/releases_q_per_page=10"

# Signing: a throwaway key for this run. install-signed.sh is install.sh
# with its public half embedded, as the release would have it.
SIGNING=""
if command -v openssl >/dev/null 2>&1; then
    openssl ecparam -name prime256v1 -genkey -noout -out "$T/key.pem" 2>/dev/null &&
        openssl ec -in "$T/key.pem" -pubout -out "$T/key.pub" 2>/dev/null &&
        openssl ecparam -name prime256v1 -genkey -noout -out "$T/other.pem" 2>/dev/null &&
        SIGNING=1
fi
if [ -n "$SIGNING" ]; then
    for v in 1.1.0 1.4.0 1.5.0; do
        openssl dgst -sha256 -sign "$T/key.pem" -out "$REL/download/v$v/SHA256SUMS.sig" "$REL/download/v$v/SHA256SUMS"
    done
    openssl dgst -sha256 -sign "$T/other.pem" -out "$REL/download/v1.2.0/SHA256SUMS.sig" "$REL/download/v1.2.0/SHA256SUMS"
    awk -v keyfile="$T/key.pub" '
        /^SIGNING_KEY=/ {
            printf "SIGNING_KEY=\""
            first = 1
            while ((getline line < keyfile) > 0) { if (!first) printf "\n"; printf "%s", line; first = 0 }
            print "\""
            # skip the rest of an existing multi-line key
            if ($0 !~ /"$/ || $0 == "SIGNING_KEY=\"") { while ((getline) > 0) if ($0 ~ /"$/) break }
            next
        }
        /^SIGNED_FROM=/ { print "SIGNED_FROM=\"v1.1.0\""; next }
        { print }
    ' "$ROOT/install.sh" >"$T/install-signed.sh"
fi

# the Rust installer: puts a stub cargo and rustc in ~/.cargo/bin
cat >"$SRV/sh.rustup.rs" <<'EOF'
#!/bin/sh
mkdir -p "$HOME/.cargo/bin"
cp "$FAKE_STUBS/cargo.real" "$HOME/.cargo/bin/cargo"
cp "$FAKE_STUBS/rustc.real" "$HOME/.cargo/bin/rustc"
chmod 755 "$HOME/.cargo/bin/cargo" "$HOME/.cargo/bin/rustc"
echo "rustup: installed Rust"
EOF

# ── stubs and the real tools a case may use ────────────────────────
SYS="$T/sys"
STUBS="$T/stubs"
mkdir -p "$SYS" "$STUBS"
for tool in sh cat cp mv rm mkdir rmdir chmod ls head tail sed grep awk tr cut wc sort \
    tar gzip mktemp dirname basename id env printf date touch ln sha256sum shasum perl openssl xattr pwd; do
    p=$(command -v "$tool" 2>/dev/null) && case "$p" in /*) ln -s "$p" "$SYS/$tool" ;; esac
done

cat >"$STUBS/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in -m) echo "$FAKE_ARCH" ;; *) echo "$FAKE_OS" ;; esac
EOF
cat >"$STUBS/ldd" <<'EOF'
#!/bin/sh
if [ "${1:-}" = --version ]; then
    if [ "${FAKE_LIBC:-glibc}" = musl ]; then echo "musl libc (aarch64)"; echo "Version 1.2.4" >&2; exit 1; fi
    echo "ldd (Ubuntu GLIBC ${FAKE_GLIBC:-2.39}-0ubuntu8) ${FAKE_GLIBC:-2.39}"
    exit 0
fi
[ -n "${FAKE_NEED_LIB:-}" ] && [ ! -f "$FAKE_STATE/lib-installed" ] && echo "	$FAKE_NEED_LIB => not found"
exit 0
EOF
cat >"$STUBS/getconf" <<'EOF'
#!/bin/sh
[ "${FAKE_LIBC:-glibc}" = glibc ] && [ "${1:-}" = GNU_LIBC_VERSION ] && { echo "glibc ${FAKE_GLIBC:-2.39}"; exit 0; }
exit 1
EOF
cat >"$STUBS/sw_vers" <<'EOF'
#!/bin/sh
echo "${FAKE_MACOS:-15.1}"
EOF
cat >"$STUBS/sysctl" <<'EOF'
#!/bin/sh
echo "${FAKE_ROSETTA:-0}"
EOF
cat >"$STUBS/id" <<EOF
#!/bin/sh
if [ "\${1:-}" = -u ] && [ -n "\${FAKE_UID:-}" ]; then echo "\$FAKE_UID"; exit 0; fi
exec "$SYS/id" "\$@"
EOF
cat >"$STUBS/apt-get" <<'EOF'
#!/bin/sh
echo "apt-get $*" >>"$FAKE_LOG"
touch "$FAKE_STATE/lib-installed"
EOF
cat >"$STUBS/sudo" <<'EOF'
#!/bin/sh
echo "sudo $*" >>"$FAKE_LOG"
exec "$@"
EOF
cat >"$STUBS/cargo.real" <<'EOF'
#!/bin/sh
echo "cargo $*" >>"$FAKE_LOG"
root=""; tag=""
while [ $# -gt 0 ]; do
    case "$1" in --root) root=$2; shift ;; --tag) tag=$2; shift ;; esac
    shift
done
mkdir -p "$root/bin"
for b in olang otc; do printf '#!/bin/sh\necho "%s %s"\n' "$b" "${tag#v}" >"$root/bin/$b"; chmod 755 "$root/bin/$b"; done
EOF
cat >"$STUBS/rustc.real" <<'EOF'
#!/bin/sh
echo "rustc ${FAKE_RUSTC:-1.99.0} (stub 2026-10-01)"
EOF
chmod 755 "$STUBS"/*

# curl: serves files from the pretend server. Refuses anything not
# called with --proto '=https' --tlsv1.2.
cat >"$STUBS/curl" <<'EOF'
#!/bin/sh
out=""; fmt=""; follow=""; failflag=""; url=""; proto=""; tls=""
while [ $# -gt 0 ]; do
    case "$1" in
        --proto) proto=$2; shift ;;
        --tlsv1.2) tls=1 ;;
        -o) out=$2; shift ;;
        -w) fmt=$2; shift ;;
        --retry|--connect-timeout) shift ;;
        --progress-bar) ;;
        --*) ;;
        -*) case "$1" in *L*) follow=1 ;; esac; case "$1" in *f*) failflag=1 ;; esac ;;
        *) url=$1 ;;
    esac
    shift
done
if [ "$proto" != "=https" ] || [ -z "$tls" ]; then
    echo "INSECURE curl $url" >>"$FAKE_LOG"; echo "stub curl: refusing a call that is not HTTPS-only: $url" >&2; exit 99
fi
echo "curl $url" >>"$FAKE_LOG"
[ -n "${FAKE_CURL_EXIT:-}" ] && exit "$FAKE_CURL_EXIT"
case "$url" in https://*) ;; *) exit 1 ;; esac
locate() { printf '%s/%s' "$FAKE_SERVER" "${1#https://}" | sed 's/?/_q_/'; }
path=$(locate "$url")
while [ -n "$follow" ] && [ -f "$path.redirect" ]; do url=$(cat "$path.redirect"); path=$(locate "$url"); done
if [ -f "$path" ]; then
    code=200
    if [ -n "$out" ]; then cp "$path" "$out"; else cat "$path"; fi
else
    code=404
    [ -n "$failflag" ] && exit 22
fi
[ -n "$fmt" ] && printf '%s' "$fmt" | sed "s#%{url_effective}#$url#; s#%{http_code}#$code#"
exit 0
EOF
# wget: the same server, GNU-style or BusyBox-style (FAKE_WGET_GNU=1)
cat >"$STUBS/wget" <<'EOF'
#!/bin/sh
if [ "${1:-}" = --help ]; then [ "${FAKE_WGET_GNU:-}" = 1 ] && echo "  --https-only   only follow secure HTTPS links"; exit 0; fi
out=""; spider=""; headers=""; url=""; safe=""
while [ $# -gt 0 ]; do
    case "$1" in
        -O) out=$2; shift ;;
        -T) shift ;;
        --spider) spider=1 ;;
        -S) headers=1 ;;
        --https-only) safe=1 ;;
        -*) ;;
        *) url=$1 ;;
    esac
    shift
done
if [ "${FAKE_WGET_GNU:-}" = 1 ] && [ -z "$safe" ]; then
    echo "INSECURE wget $url" >>"$FAKE_LOG"; exit 99
fi
echo "wget $url" >>"$FAKE_LOG"
case "$url" in https://*) ;; *) exit 1 ;; esac
locate() { printf '%s/%s' "$FAKE_SERVER" "${1#https://}" | sed 's/?/_q_/'; }
path=$(locate "$url")
while [ -f "$path.redirect" ]; do
    url=$(cat "$path.redirect")
    [ -n "$headers" ] && echo "  Location: $url" >&2
    path=$(locate "$url")
done
[ -f "$path" ] || exit 8
[ -n "$spider" ] && exit 0
cp "$path" "$out"
EOF
chmod 755 "$STUBS/curl" "$STUBS/wget"

# ── running a case ─────────────────────────────────────────────────
# run NAME [VAR=value...] -- [install.sh options...]
# Runs the installer as a pretend computer described by the variables
# (FAKE_OS, FAKE_ARCH, FAKE_LIBC, FAKE_GLIBC, ...). Sets OUT (stdout and
# stderr together), CODE, LOG (downloads and commands run) and H (HOME).
# `again` is `run` in the HOME of the case before.
SCRIPT="$ROOT/install.sh"
run() {
    NAME=$1; shift
    if [ "${KEEP:-}" != 1 ]; then H="$T/home/$NAME"; rm -rf "$H"; mkdir -p "$H"; fi
    KEEP=""
    STATE="$T/state/$NAME"; rm -rf "$STATE"; mkdir -p "$STATE"
    LOGF="$STATE/log"; : >"$LOGF"
    vars=""
    while [ $# -gt 0 ] && [ "$1" != -- ]; do vars="$vars $1"; shift; done
    [ $# -gt 0 ] && shift
    path="$STUBS:$SYS"
    case "$vars" in *NO_CURL=1*) mkdir -p "$STATE/bin"; for f in "$STUBS"/*; do case "${f##*/}" in curl|wget) ;; *) ln -s "$f" "$STATE/bin/";; esac; done
        case "$vars" in *NO_WGET=1*) ;; *) ln -s "$STUBS/wget" "$STATE/bin/" ;; esac; path="$STATE/bin:$SYS" ;; esac
    case "$vars" in *NO_OPENSSL=1*) mkdir -p "$STATE/sys"; for f in "$SYS"/*; do [ "${f##*/}" = openssl ] || ln -s "$f" "$STATE/sys/"; done; path="${path%"$SYS"}$STATE/sys" ;; esac
    case "$vars" in *WITH_CARGO=1*) mkdir -p "$STATE/cargo"; ln -s "$STUBS/cargo.real" "$STATE/cargo/cargo"; ln -s "$STUBS/rustc.real" "$STATE/cargo/rustc"; path="$STATE/cargo:$path" ;; esac
    case "$vars" in *ON_PATH=1*) path="$H/.olang/bin:$path" ;; esac
    case "$vars" in *OTHER_OLANG=1*) mkdir -p "$STATE/other"; printf '#!/bin/sh\necho "olang 0.1.0"\n' >"$STATE/other/olang"
        chmod 755 "$STATE/other/olang"; path="$STATE/other:$path" ;; esac
    # shellcheck disable=SC2086 # vars is a list of VAR=value words
    OUT=$(cd "$H" && env -i HOME="$H" PATH="$path" SHELL=/bin/zsh FAKE_OS=Darwin FAKE_ARCH=arm64 \
        FAKE_SERVER="$SRV" FAKE_LOG="$LOGF" FAKE_STATE="$STATE" FAKE_STUBS="$STUBS" OLANG_YES=0 \
        $vars "$SHELL_BIN" "$SCRIPT" "$@" 2>&1 </dev/null)
    CODE=$?
    LOG=$(cat "$LOGF")
}
again() { KEEP=1; run "$@"; }

ok_()  { PASS=$((PASS + 1)); }
bad()  { FAILED=$((FAILED + 1)); printf 'FAIL [%s] %s: %s\n' "$SHELL_NAME" "$NAME" "$1"; printf '%s\n' "$OUT" | sed 's/^/    | /'; }
code() { [ "$CODE" = "$1" ] && ok_ || bad "exit code $CODE, expected $1"; }
has()  { printf '%s\n' "$OUT" | grep -qF -- "$1" && ok_ || bad "output lacks: $1"; }
lacks() { printf '%s\n' "$OUT" | grep -qF -- "$1" && bad "output has: $1" || ok_; }
logged() { printf '%s\n' "$LOG" | grep -qF -- "$1" && ok_ || bad "log lacks: $1"; }
unlogged() { printf '%s\n' "$LOG" | grep -qF -- "$1" && bad "log has: $1" || ok_; }
file() { [ -e "$1" ] && ok_ || bad "missing file $1"; }
nofile() { [ -e "$1" ] && bad "file should not exist: $1" || ok_; }
installed() { # VERSION: olang and otc in ~/.olang/bin at that version
    [ "$("$H/.olang/bin/olang" --version 2>/dev/null)" = "olang $1" ] &&
        [ "$("$H/.olang/bin/otc" --version 2>/dev/null)" = "otc $1" ] && ok_ ||
        bad "olang $1 is not installed in ~/.olang/bin"
}
secure() { printf '%s\n' "$LOG" | grep -q INSECURE && bad "a download was not HTTPS-only" || ok_; }
fetched() { logged "releases/download/v$1/olang-$1-$2.tar.gz"; }

# ── macOS ──────────────────────────────────────────────────────────
run mac-arm64 --
code 0; installed 1.1.0; fetched 1.1.0 macos-arm64; secure
has "macOS, Apple silicon, 15.1"; has "1.1.0 (the newest)"; has "Installed olang and otc"
has "Added ~/.olang/bin to your PATH in ~/.zshrc"; has "olang is ready."; has "Open a new terminal window"
grep -qxF 'export PATH="$HOME/.olang/bin:$PATH"' "$H/.zshrc" && ok_ || bad ".zshrc lacks the PATH line"
printf '%s\n' "$OUT" | grep -q "$(printf '\033')" && bad "colour codes without a terminal" || ok_

again mac-arm64-again --
code 0; has "olang 1.1.0 is already installed here"; has ".zshrc already adds ~/.olang/bin"
unlogged "tar.gz"
[ "$(grep -c 'olang/bin' "$H/.zshrc")" = 1 ] && ok_ || bad "the PATH line was added twice"

run mac-update -- --version v1.0.0
code 0; installed 1.0.0; lacks "(the newest)"
again mac-update-2 --
code 0; installed 1.1.0; has "Updated olang and otc from 1.0.0 to 1.1.0"

again mac-force -- --force
code 0; fetched 1.1.0 macos-arm64; has "Reinstalled olang 1.1.0"

run mac-rosetta FAKE_ARCH=x86_64 FAKE_ROSETTA=1 --
code 0; fetched 1.1.0 macos-arm64; has "Apple silicon"

run mac-intel FAKE_ARCH=x86_64 FAKE_MACOS=12.7 --
code 0; fetched 1.1.0 macos-x64; has "macOS, Intel, 12.7"

run mac-version-eq -- --version=1.0.0
code 0; installed 1.0.0
run mac-version-env OLANG_VERSION=v1.0.0 --
code 0; installed 1.0.0

# ── Linux ──────────────────────────────────────────────────────────
run linux-x64 FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_GLIBC=2.35 SHELL=/bin/bash --
code 0; installed 1.1.0; fetched 1.1.0 linux-x64; has "Linux, x86_64"; lacks "gui.available"
file "$H/.bashrc"; nofile "$H/.zshrc"; has "in ~/.bashrc"

run linux-arm64 FAKE_OS=Linux FAKE_ARCH=aarch64 --
code 0; fetched 1.1.0 linux-arm64

echo "Linux version 5.15.153.1-microsoft-standard-WSL2" >"$T/procversion"
run linux-wsl FAKE_OS=Linux FAKE_ARCH=x86_64 _OLANG_PROC_VERSION="$T/procversion" --
code 0; has "Linux (WSL), x86_64"; fetched 1.1.0 linux-x64

run linux-old-glibc FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_GLIBC=2.17 --
code 0; installed 1.1.0; fetched 1.1.0 linux-x64-musl; unlogged "linux-x64.tar.gz"
has "older than olang's standard build supports (glibc 2.28)"

run linux-musl FAKE_OS=Linux FAKE_ARCH=aarch64 FAKE_LIBC=musl --
code 0; installed 1.1.0; fetched 1.1.0 linux-arm64-musl; has "Linux, aarch64, musl"; has "gui.available() is false"

# getconf missing the answer: the glibc build fails at its trial run,
# and the static build takes its place
run linux-glibc-fallback FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_FAIL_PLATFORM=linux-x64 --
code 0; installed 1.1.0; fetched 1.1.0 linux-x64; fetched 1.1.0 linux-x64-musl; has "static build"

run linux-glibc-no-static FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_FAIL_PLATFORM=linux-x64 -- --version v1.5.0
code 1; has "needs glibc 2.28 or newer"; has "needs Rust"; nofile "$H/.olang/bin/olang"

run linux-old-glibc-no-static FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_GLIBC=2.17 -- --version v1.5.0
code 1; has "this Linux has glibc 2.17, older than the 2.28"

run linux-static-missing FAKE_OS=Linux FAKE_ARCH=aarch64 FAKE_LIBC=musl -- --version v0.9.0
code 1; has "release v0.9.0 has no ready-made download for Linux, aarch64, musl"

run linux-lib-yes FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_NEED_LIB=libgcc_s.so.1 OLANG_YES=1 FAKE_UID=1000 --
code 0; installed 1.1.0; logged "sudo apt-get install -y libgcc-s1"; has "Installed libgcc-s1"

run linux-lib-no FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_NEED_LIB=libgcc_s.so.1 FAKE_UID=1000 --
code 1; has "olang needs libgcc-s1"; has "sudo apt-get install -y libgcc-s1"; unlogged "apt-get install"
nofile "$H/.olang/bin/olang"

run linux-lib-unknown FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_NEED_LIB=libweird.so.3 --
code 1; has "a system library this computer does not have: libweird.so.3"

run linux-armv7 FAKE_OS=Linux FAKE_ARCH=armv7l --
code 1; has "32-bit armv7l"; has "64-bit Raspberry Pi OS"; has "a source build needs Rust"; unlogged "tar.gz"

run linux-fish FAKE_OS=Linux FAKE_ARCH=x86_64 SHELL=/usr/bin/fish --
code 0; file "$H/.config/fish/conf.d/olang.fish"; grep -qF 'fish_add_path "$HOME/.olang/bin"' "$H/.config/fish/conf.d/olang.fish" && ok_ || bad "fish line"
again linux-fish-uninstall FAKE_OS=Linux FAKE_ARCH=x86_64 SHELL=/usr/bin/fish -- --uninstall
code 0; nofile "$H/.config/fish/conf.d/olang.fish"; nofile "$H/.olang/bin/olang"

# ── other systems ──────────────────────────────────────────────────
run windows FAKE_OS=MINGW64_NT-10.0-22631 FAKE_ARCH=x86_64 --
code 1; has "irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex"; has "wsl --install"; unlogged "curl"

run freebsd-no-rust FAKE_OS=FreeBSD FAKE_ARCH=amd64 --
code 1; has "there is no ready-made olang for FreeBSD"; has "Rust is not installed"
has "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"

run freebsd-rustup FAKE_OS=FreeBSD FAKE_ARCH=amd64 OLANG_YES=1 --
code 0; installed 1.1.0; logged "curl https://sh.rustup.rs"; has "rustup: installed Rust"
logged "cargo install --locked --git https://github.com/ooyeku/olang.git --tag v1.1.0"; secure

run freebsd-cargo FAKE_OS=FreeBSD FAKE_ARCH=amd64 OLANG_YES=1 WITH_CARGO=1 --
code 0; installed 1.1.0; unlogged "sh.rustup.rs"

run freebsd-cargo-declined FAKE_OS=FreeBSD FAKE_ARCH=amd64 WITH_CARGO=1 --
code 1; has "the source build was not started"; has "cargo install --locked --git https://github.com/ooyeku/olang.git --tag v1.1.0 olang otc"

run old-rust FAKE_OS=FreeBSD FAKE_ARCH=amd64 OLANG_YES=1 WITH_CARGO=1 FAKE_RUSTC=1.80.0 --
code 1; has "this computer has 1.80, which rustup does not manage"

run from-source OLANG_YES=1 WITH_CARGO=1 -- --from-source
code 0; installed 1.1.0; has "you asked for a source build"; unlogged "tar.gz"

run no-downloads-in-release -- --version v0.8.0
code 1; has "release v0.8.0 has no SHA256SUMS file"

# ── downloads and checks ───────────────────────────────────────────
run checksum-mismatch -- --version v1.4.0
code 1; has "did not match its published checksum"; nofile "$H/.olang/bin/olang"

run offline FAKE_CURL_EXIT=6 --
code 1; has "could not reach GitHub"; has "Check that this computer is online"

run tls-broken FAKE_CURL_EXIT=60 --
code 1; has "secure connection to GitHub could not be verified"

run no-such-version -- --version v9.9.9
code 1; has "there is no olang release called v9.9.9"

run bad-version -- --version banana
code 1; has "'banana' is not a version number"

echo "https://github.com/ooyeku/olang/releases" >"$REL/latest.redirect"
run all-prereleases --
code 0; installed 1.1.0; logged "api.github.com/repos/ooyeku/olang/releases?per_page=10"
mv "$SRV/api.github.com/repos/ooyeku/olang/releases_q_per_page=10" "$T/api"
run rate-limited --
code 1; has "GitHub would not list olang's releases just now"
mv "$T/api" "$SRV/api.github.com/repos/ooyeku/olang/releases_q_per_page=10"
echo "https://github.com/ooyeku/olang/releases/tag/v1.1.0" >"$REL/latest.redirect"

run wget-busybox NO_CURL=1 FAKE_OS=Linux FAKE_ARCH=aarch64 FAKE_LIBC=musl --
code 0; installed 1.1.0; logged "wget https://github.com/ooyeku/olang/releases/latest"; unlogged "curl "
run wget-gnu NO_CURL=1 FAKE_WGET_GNU=1 FAKE_OS=Linux FAKE_ARCH=x86_64 -- --version v1.0.0
code 0; installed 1.0.0; secure
run wget-missing-version NO_CURL=1 -- --version v9.9.9
code 1; has "there is no olang release called v9.9.9"
run no-downloader NO_CURL=1 NO_WGET=1 --
code 1; has "neither 'curl' nor 'wget'"

# ── signatures ─────────────────────────────────────────────────────
if [ -n "$SIGNING" ]; then
    SCRIPT="$T/install-signed.sh"
    run sig-good --
    code 0; installed 1.1.0; has "Signature verified"; logged "v1.1.0/SHA256SUMS.sig"; secure
    run sig-bad -- --version v1.2.0
    code 1; has "signature does not match olang's signing key"; nofile "$H/.olang/bin/olang"; unlogged "tar.gz"
    run sig-missing -- --version v1.3.0
    code 1; has "release v1.3.0 has no signature"; nofile "$H/.olang/bin/olang"
    run sig-missing-allowed OLANG_ALLOW_UNSIGNED=1 -- --version v1.3.0
    code 0; installed 1.3.0; has "Going on because OLANG_ALLOW_UNSIGNED=1"
    run sig-older-release -- --version v1.0.0
    code 0; installed 1.0.0; has "older than olang's release signatures (they start at v1.1.0)"
    run sig-no-openssl NO_OPENSSL=1 --
    code 0; installed 1.1.0; has "signature was not checked, because this computer has no 'openssl'"
    run sig-no-openssl-required NO_OPENSSL=1 OLANG_REQUIRE_SIGNATURE=1 --
    code 1; has "the release signature cannot be checked"
    run sig-signed-then-bad-sum -- --version v1.4.0
    code 1; has "did not match its published checksum"
    SCRIPT="$ROOT/install.sh"
    run sig-unset --
    code 0; lacks "Signature"; unlogged "SHA256SUMS.sig"
else
    echo "note: no openssl here, so the signature cases were skipped" >&2
fi

# ── options, folders and PATH ──────────────────────────────────────
run help -- --help
code 0; has "Usage:"; has "--uninstall"; unlogged "curl"

run unknown-option -- --frobnicate
code 1; has "does not know the option '--frobnicate'"

run missing-value -- --version
code 1; has "the option --version needs a value"

run quiet -- --quiet
code 0; installed 1.1.0
[ -z "$OUT" ] && ok_ || bad "quiet mode printed something"

run quiet-failure FAKE_CURL_EXIT=6 -- -q
code 1; has "could not reach GitHub"

run no-modify-path -- --no-modify-path
code 0; nofile "$H/.zshrc"; has "add this line to ~/.zshrc"; has 'export PATH="$HOME/.olang/bin:$PATH"'

run custom-dir -- --dir "$T/home/custom-dir/tools/bin"
code 0; file "$T/home/custom-dir/tools/bin/olang"; has "~/tools/bin"
grep -qF '$HOME/tools/bin' "$H/.zshrc" && ok_ || bad "custom dir PATH line"

run under-sudo FAKE_UID=0 SUDO_USER=someone --
code 1; has "run with sudo, which it does not need"; unlogged "tar.gz"

run root-container FAKE_UID=0 FAKE_OS=Linux FAKE_ARCH=x86_64 --
code 0; installed 1.1.0

if [ "$(id -u)" != 0 ]; then   # root can write anywhere
    H="$T/home/locked"; mkdir -p "$H/.olang/bin"; chmod 555 "$H/.olang/bin"
    again locked --
    code 1; has "you do not have permission to write to ~/.olang/bin"; has "sudo chown -R"
    chmod 755 "$H/.olang/bin"
fi

run on-path ON_PATH=1 --
code 0; nofile "$H/.zshrc"; has "olang is ready. Run:"; lacks "Open a new terminal"

run other-olang-first ON_PATH=1 OTHER_OLANG=1 --
code 0; has "Another olang, at"; has "cargo uninstall olang otc"

# ── uninstall ──────────────────────────────────────────────────────
run uninstall-setup --
printf 'alias ll="ls -l"\n' >"$H/.zshrc.before"
{ printf 'alias ll="ls -l"\n'; cat "$H/.zshrc"; printf 'export EDITOR=vi\n'; } >"$H/.zshrc.new" && mv "$H/.zshrc.new" "$H/.zshrc"
mkdir -p "$H/.olang/shelf"; echo pkg >"$H/.olang/shelf/x"
again uninstall -- --uninstall
code 0; nofile "$H/.olang/bin/olang"; nofile "$H/.olang/bin/otc"; file "$H/.olang/shelf/x"
has "Removed olang and otc from ~/.olang/bin"; has "Removed the PATH line from ~/.zshrc"; has "were kept"
[ "$(cat "$H/.zshrc")" = "$(printf 'alias ll="ls -l"\nexport EDITOR=vi')" ] && ok_ || bad "uninstall changed other lines of .zshrc: $(cat "$H/.zshrc")"
again uninstall-yes OLANG_YES=1 -- --uninstall
code 0; has "olang was not installed in ~/.olang/bin"; file "$H/.olang/shelf/x"

printf '%s passed, %s failed (%s)\n' "$PASS" "$FAILED" "$SHELL_NAME"
[ "$FAILED" = 0 ]
