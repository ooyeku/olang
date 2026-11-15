#!/bin/bash
# tools/release-check.sh — prove a published olang release installs and runs.
#
#   tools/release-check.sh [TAG | latest] [options]
#
# Read-only against GitHub: it downloads, verifies and installs into a
# temporary folder with its own HOME, and changes nothing anywhere else.
# For TAG (default: latest, the release the installers pick) it
#
#   1. downloads SHA256SUMS, SHA256SUMS.sig and this computer's tarball;
#   2. verifies the signature on SHA256SUMS with olang's release key
#      (dist/signing/olang-release.pub, else install.sh's SIGNING_KEY), and
#      the tarball's checksum against SHA256SUMS; with gh, its build
#      attestation too;
#   3. installs it the way a person does — the published install.sh from
#      main, `--version TAG` — into the temporary HOME;
#   4. smoke-tests what was installed: the versions, a program, `olang
#      check`, `olang test`, and the compatibility programs
#      (tools/compat-check.sh) with the installed olang;
#   5. checks an upgrade: installs the release before TAG, then TAG over
#      it with the same installer, and sees the version move.
#
# Options:
#   --all                    download and checksum every file, not just this platform's
#   --installer local        run this checkout's install.sh, not main's on GitHub
#   --upgrade-from TAG|none  the release to upgrade from (default: the one before)
#   --no-compat              skip the compatibility programs
#   --require-signature      fail when the release is unsigned, even before a key is pinned
#   --keep                   keep the temporary folder, to look around
#
# Exit 0 when the release checks out, 1 when something is wrong with it,
# 2 when it could not be checked (no network, bad arguments).
set -uo pipefail
cd "$(dirname "$0")/.."
# semver_gt A B: A is newer than B (2.10.0 > 2.9.0, 1.0.0 > 1.0.0-rc.9,
# rc.10 > rc.9), as install.sh orders releases.
semver_gt() {
    local a=${1#v} b=${2#v}
    [ "$a" = "$b" ] && return 1
    local ac=${a%%-*} bc=${b%%-*} ap="" bp=""
    case "$a" in *-*) ap=${a#*-} ;; esac
    case "$b" in *-*) bp=${b#*-} ;; esac
    local i x y
    for i in 1 2 3; do
        x=$(echo "$ac" | cut -d. -f$i) y=$(echo "$bc" | cut -d. -f$i)
        [ "$x" -gt "$y" ] && return 0
        [ "$x" -lt "$y" ] && return 1
    done
    [ -z "$ap" ] && [ -n "$bp" ] && return 0
    [ -n "$ap" ] && [ -z "$bp" ] && return 1
    # both prereleases of one version: compare dot by dot, numbers as numbers
    local IFS=.
    local -a pa=($ap) pb=($bp)
    for ((i = 0; i < ${#pa[@]} || i < ${#pb[@]}; i++)); do
        x=${pa[i]:-} y=${pb[i]:-}
        [ -z "$x" ] && return 1
        [ -z "$y" ] && return 0
        if [[ $x =~ ^[0-9]+$ && $y =~ ^[0-9]+$ ]]; then
            [ "$x" -gt "$y" ] && return 0
            [ "$x" -lt "$y" ] && return 1
        else
            [[ $x > $y ]] && return 0
            [[ $x < $y ]] && return 1
        fi
    done
    return 1
}

REPO=ooyeku/olang
GITHUB=https://github.com
RAW=https://raw.githubusercontent.com/$REPO

TAG=latest ALL=0 INSTALLER=published UPGRADE_FROM=auto COMPAT=1 REQUIRE_SIG=0 KEEP=0
while [ $# -gt 0 ]; do
    case "$1" in
        --all) ALL=1 ;;
        --installer) INSTALLER=${2:?--installer is local or published}; shift ;;
        --upgrade-from) UPGRADE_FROM=${2:?--upgrade-from needs a tag or none}; shift ;;
        --no-compat) COMPAT=0 ;;
        --require-signature) REQUIRE_SIG=1 ;;
        --keep) KEEP=1 ;;
        -h|--help) sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "release-check: unknown option $1 (see --help)" >&2; exit 2 ;;
        *) TAG=$1 ;;
    esac
    shift
done
case "$INSTALLER" in local|published) ;; *) echo "release-check: --installer is local or published" >&2; exit 2 ;; esac

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then B=$'\e[1m' R=$'\e[31m' G=$'\e[32m' Y=$'\e[33m' D=$'\e[2m' N=$'\e[0m'
else B="" R="" G="" Y="" D="" N=""; fi
FAILS=0 WARNS=0
step() { printf '\n%s%s%s\n' "$B" "$*" "$N"; }
ok() { printf '  %s✓%s %s\n' "$G" "$N" "$*"; }
warn() { printf '  %s!%s %s\n' "$Y" "$N" "$*"; WARNS=$((WARNS + 1)); }
bad() { printf '  %s✗ %s%s\n' "$R" "$*" "$N"; FAILS=$((FAILS + 1)); }
cannot() { printf '%srelease-check: %s%s\n' "$R" "$*" "$N" >&2; exit 2; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/olang-release-check.XXXXXX") || cannot "no temporary folder"
if [ $KEEP -eq 1 ]; then trap 'echo "kept: $WORK"' EXIT; else trap 'rm -rf "$WORK"' EXIT; fi
DL=$WORK/download        # what came from the release, untrusted until checked
mkdir -p "$DL" "$WORK/installer" "$WORK/home" "$WORK/home-upgrade" "$WORK/smoke"

# fetch URL FILE: 0 got it, 22 not found (HTTP 4xx), else no network.
# The status decides, since curl's own exit code for a 404 after a
# redirect varies between versions (22 or 56).
fetch() {
    local code
    code=$(curl -sSL --proto '=https' --tlsv1.2 --retry 2 -o "$2" -w '%{http_code}' "$1" 2>/dev/null) || code=${code:-000}
    case "$code" in
        2??) return 0 ;;
        4??) rm -f "$2"; return 22 ;;
        *) rm -f "$2"; return 7 ;;
    esac
}

# ── which release ────────────────────────────────────────────────────
if [ "$TAG" = latest ]; then
    url=$(curl -fsSLI --proto '=https' -o /dev/null -w '%{url_effective}' "$GITHUB/$REPO/releases/latest") ||
        cannot "could not reach $GITHUB (offline?)"
    case "$url" in */releases/tag/v*) TAG=${url##*/releases/tag/} ;; *) cannot "GitHub has no latest release for $REPO" ;; esac
    LATEST_NOTE=" (the latest, as the installers see it)"
else
    case "$TAG" in v*) ;; *) TAG=v$TAG ;; esac
    LATEST_NOTE=""
fi
case "$TAG" in *[!A-Za-z0-9.-]*) cannot "'$TAG' is not a release tag" ;; esac
VERSION=${TAG#v}
printf '%sChecking olang %s%s%s\n' "$B" "$TAG" "$LATEST_NOTE" "$N"

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) PLATFORM=macos-arm64 ;;
    Darwin-x86_64) PLATFORM=macos-x64 ;;
    Linux-x86_64) PLATFORM=linux-x64 ;;
    Linux-aarch64|Linux-arm64) PLATFORM=linux-arm64 ;;
    *) cannot "no release download for $(uname -s) $(uname -m)" ;;
esac
TARBALL=olang-$VERSION-$PLATFORM.tar.gz
BASE=$GITHUB/$REPO/releases/download/$TAG

# ── 1. download ──────────────────────────────────────────────────────
step "1. Download"
fetch "$BASE/SHA256SUMS" "$DL/SHA256SUMS"
case $? in 0) ok "SHA256SUMS ($(wc -l <"$DL/SHA256SUMS" | tr -d ' ') files listed)" ;;
    22) bad "the release $TAG has no SHA256SUMS (or no such release: $GITHUB/$REPO/releases/tag/$TAG)"; exit 1 ;;
    *) cannot "could not download $BASE/SHA256SUMS" ;; esac
fetch "$BASE/SHA256SUMS.sig" "$DL/SHA256SUMS.sig"; sig_rc=$?
[ $sig_rc -eq 0 ] && ok "SHA256SUMS.sig" || { rm -f "$DL/SHA256SUMS.sig"; [ $sig_rc -eq 22 ] && echo "  - no SHA256SUMS.sig on the release" || cannot "could not download SHA256SUMS.sig"; }
if [ $ALL -eq 1 ]; then FILES=$(awk '{ sub(/^\*/, "", $2); print $2 }' "$DL/SHA256SUMS"); else FILES=$TARBALL; fi
for f in $FILES; do
    case "$f" in */*|..*) bad "SHA256SUMS lists a strange name: $f"; continue ;; esac
    if fetch "$BASE/$f" "$DL/$f"; then ok "$f ($(du -h "$DL/$f" | cut -f1 | tr -d ' '))"; else bad "could not download $f"; fi
done

# ── 2. verify ────────────────────────────────────────────────────────
step "2. Verify"
# the release key: the file of record, else the copy install.sh pins
KEY=$WORK/olang-release.pub
if [ -s dist/signing/olang-release.pub ]; then cp dist/signing/olang-release.pub "$KEY"; KEY_FROM=dist/signing/olang-release.pub
else
    sed -n '/^SIGNING_KEY="-----BEGIN PUBLIC KEY-----/,/-----END PUBLIC KEY-----"$/p' install.sh | sed 's/^SIGNING_KEY="//; s/"$//' >"$KEY"
    KEY_FROM=install.sh
fi
SIGNED_FROM=$(sed -n 's/^SIGNED_FROM="\(.*\)"$/\1/p' install.sh)
if [ ! -s "$KEY" ]; then
    if [ $REQUIRE_SIG -eq 1 ]; then bad "no release key is pinned (OL-491), so nothing can check $TAG's signature"
    else warn "no release key is pinned yet (OL-491): the signature cannot be checked, so $TAG is checked by checksum only"; fi
elif [ ! -f "$DL/SHA256SUMS.sig" ]; then
    # older than the first signed release: checksum only, as install.sh does
    if [ -n "$SIGNED_FROM" ] && semver_gt "$SIGNED_FROM" "$TAG" && [ $REQUIRE_SIG -eq 0 ]; then
        warn "$TAG is older than the first signed release ($SIGNED_FROM): checksum only"
    else bad "$TAG has no SHA256SUMS.sig, and every release from ${SIGNED_FROM:-the first signed one} on must be signed"; fi
elif openssl dgst -sha256 -verify "$KEY" -signature "$DL/SHA256SUMS.sig" "$DL/SHA256SUMS" >/dev/null 2>&1; then
    ok "SHA256SUMS.sig verifies with olang's release key ($KEY_FROM, ECDSA P-256)"
else
    bad "SHA256SUMS.sig does NOT verify with the release key in $KEY_FROM: do not install this release; see RELEASING.md, Rolling back"
fi
for f in $FILES; do
    [ -f "$DL/$f" ] || continue
    want=$(awk -v f="$f" '{ n = $2; sub(/^\*/, "", n) } n == f { print $1 }' "$DL/SHA256SUMS")
    got=$( (shasum -a 256 "$DL/$f" 2>/dev/null || sha256sum "$DL/$f") | cut -d' ' -f1)
    if [ -z "$want" ]; then bad "$f is not listed in SHA256SUMS"
    elif [ "$want" = "$got" ]; then ok "$f matches its SHA256SUMS line"
    else bad "$f does not match SHA256SUMS (want $want, got $got)"; fi
done
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
    if gh attestation verify "$DL/$TARBALL" --repo "$REPO" >"$WORK/attest.log" 2>&1; then
        ok "$TARBALL has a GitHub build attestation from $REPO's release workflow"
    else bad "$TARBALL's build attestation does not verify: $(tail -1 "$WORK/attest.log")"; fi
else
    echo "  - gh is not signed in: build attestation not checked"
fi

# ── 3. install, as a person does ─────────────────────────────────────
step "3. Install into a clean, temporary HOME"
if [ "$INSTALLER" = local ]; then cp install.sh "$WORK/installer/install.sh"; INSTALLER_FROM="this checkout's install.sh"
else
    fetch "$RAW/main/install.sh" "$WORK/installer/install.sh" || cannot "could not download $RAW/main/install.sh"
    INSTALLER_FROM="install.sh from main"
fi
# install HOME_DIR TAG: run the installer with its own HOME, nothing else touched
install_into() {
    (cd "$WORK" && env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin HOME="$1" TERM=dumb NO_COLOR=1 \
        sh "$WORK/installer/install.sh" --version "$2" --dir "$1/.olang/bin" --no-modify-path --yes)
}
if install_into "$WORK/home" "$TAG" >"$WORK/install.log" 2>&1; then
    ok "$INSTALLER_FROM installed $TAG into a temporary ~/.olang/bin"
    # the installer's own steps (`ok …` and `! …` lines with no colour)
    grep -E '^  (ok|!|✓) ' "$WORK/install.log" | sed 's/^  /      installer: /' | head -8
else
    bad "$INSTALLER_FROM failed to install $TAG:"; sed 's/^/      /' "$WORK/install.log" | tail -15
fi
BIN=$WORK/home/.olang/bin

# ── 4. smoke test ────────────────────────────────────────────────────
step "4. Smoke test the installed olang"
if [ -x "$BIN/olang" ]; then
    run() { env -i PATH="$BIN:/usr/bin:/bin" HOME="$WORK/home" NO_COLOR=1 "$@"; }
    v=$(run olang --version 2>&1); [ "$v" = "olang $VERSION" ] && ok "olang --version: $v" || bad "olang --version says '$v', not 'olang $VERSION'"
    v=$(run otc --version 2>&1); case "$v" in *"$VERSION"*) ok "otc --version: $v" ;; *) bad "otc --version says '$v'" ;; esac
    cat >"$WORK/smoke/hello.ol" <<'EOF'
fn greet(name) = `hello, ${name}`
let xs = [3, 1, 2] |> sort |> map((n) => n * 10)
println(greet("release"))
println(xs)
println(json.parse("{\"ok\": true}"))
test "smoke" { assert_eq(1 + 1, 2) }
EOF
    out=$(cd "$WORK/smoke" && run olang hello.ol 2>&1)
    [ "$out" = $'hello, release\n[10, 20, 30]\nOk({ ok: true })' ] && ok "a program runs" || bad "the smoke program printed: $out"
    (cd "$WORK/smoke" && run olang check hello.ol >/dev/null 2>&1) && ok "olang check" || bad "olang check failed on the smoke program"
    (cd "$WORK/smoke" && run olang test hello.ol >/dev/null 2>&1) && ok "olang test" || bad "olang test failed on the smoke program"
    out=$(run olang eval '1 + 1' 2>&1); [ "$out" = 2 ] && ok "olang eval" || bad "olang eval '1 + 1' printed $out"
    if [ $COMPAT -eq 1 ]; then
        if tools/compat-check.sh --olang "$BIN/olang" >"$WORK/compat.log" 2>&1; then
            passed=$(grep -c 'programs/' "$WORK/compat.log")
            ok "the compatibility programs pass ($passed of tests/compat/programs)"
        else
            n=$(grep -c ' ✗' "$WORK/compat.log")
            bad "$n compatibility check(s) fail with the installed $TAG (tools/compat-check.sh):"
            grep -A3 '✗' "$WORK/compat.log" | head -30 | sed 's/^/    /'
        fi
    fi
else
    bad "nothing was installed to smoke-test"
fi

# ── 5. upgrade ───────────────────────────────────────────────────────
step "5. Upgrade from the release before"
if [ "$UPGRADE_FROM" = auto ]; then
    UPGRADE_FROM=$(curl -fsSL --proto '=https' "https://api.github.com/repos/$REPO/releases?per_page=30" 2>/dev/null |
        tr ',' '\n' | sed -n 's/.*"tag_name": *"\(v[^"]*\)".*/\1/p' |
        awk -v t="$TAG" 'seen { print; exit } $0 == t { seen = 1 }')
    [ -n "$UPGRADE_FROM" ] || UPGRADE_FROM=none
fi
if [ "$UPGRADE_FROM" = none ]; then
    echo "  - no earlier release to upgrade from"
elif install_into "$WORK/home-upgrade" "$UPGRADE_FROM" >"$WORK/upgrade-old.log" 2>&1; then
    old=$("$WORK/home-upgrade/.olang/bin/olang" --version 2>&1)
    if install_into "$WORK/home-upgrade" "$TAG" >"$WORK/upgrade.log" 2>&1; then
        new=$("$WORK/home-upgrade/.olang/bin/olang" --version 2>&1)
        [ "$new" = "olang $VERSION" ] && ok "upgraded $old → $new with the same installer" || bad "after upgrading from $UPGRADE_FROM the installed olang says '$new'"
    else bad "upgrading $UPGRADE_FROM → $TAG failed:"; tail -10 "$WORK/upgrade.log" | sed 's/^/      /'; fi
else
    warn "could not install $UPGRADE_FROM to upgrade from: $(tail -3 "$WORK/upgrade-old.log" | tr '\n' ' ')"
fi

# ── verdict ──────────────────────────────────────────────────────────
echo
if [ $FAILS -eq 0 ]; then
    printf '%srelease-check: %s checks out%s' "$G$B" "$TAG" "$N"
    [ $WARNS -eq 0 ] && echo || echo " ${Y}(with $WARNS warning$([ $WARNS -eq 1 ] || echo s))${N}"
    exit 0
fi
printf '%srelease-check: %s has %d problem%s%s (RELEASING.md, "Rolling back a bad release")\n' "$R$B" "$TAG" "$FAILS" "$([ $FAILS -eq 1 ] || echo s)" "$N"
exit 1
