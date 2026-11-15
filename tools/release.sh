#!/bin/bash
# tools/release.sh — cut an olang release, up to a local tag.
#
#   tools/release.sh <version> [-m "one-line summary"] [--dry-run] [--allow-unsigned]
#
# RELEASING.md is the whole path; this script is its scripted middle. It
#
#   1. checks before anything changes: on main, a clean tree, level with
#      origin/main, a new version above the last, an [Unreleased]
#      CHANGELOG section with something in it, the release signing key
#      pinned in every copy and its secret set, the compatibility
#      allow-list's entries for this version named in the CHANGELOG, and
#      the gates CI runs (fmt, clippy, the test suite, the examples and
#      tools/compat-check.sh on the olang the tests built);
#   2. bumps the version everywhere it lives (Cargo.toml, playground,
#      Cargo.lock), opens the CHANGELOG section and its compare links,
#      and for a new minor writes its stdlib surface snapshot;
#   3. commits that as "<version>: <summary>" and makes the annotated tag
#      v<version> — locally. It never pushes, publishes or makes keys.
#
# Then it prints the exact commands that publish the release.
#
# --dry-run runs every check and shows the bump, then puts the tree back.
# --allow-unsigned lets a release candidate (a version with a suffix) go
# without the signing key; a final release never can.
#
# Cargo: $OLANG_RELEASE_CARGO (default `cargo`), for a wrapper that
# serializes builds. The examples and the compatibility check run on the
# olang the test gate built (${CARGO_TARGET_DIR:-target}/fasttest/olang,
# optimized as release is, without its LTO), or $OLANG_RELEASE_BIN: the
# release binaries are the release workflow's to build. OLANG_RELEASE_BUILD=1
# also builds them here (--release, LTO) and runs the checks on that.
# OLANG_RELEASE_SKIP_GATES=1 skips the cargo gates, for trying this
# script, and is refused without --dry-run.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO=ooyeku/olang
CARGO=${OLANG_RELEASE_CARGO:-cargo}
BRANCH=${OLANG_RELEASE_BRANCH:-main}

VERSION="" SUMMARY="" DRY=0 UNSIGNED_OK=0
while [ $# -gt 0 ]; do
    case "$1" in
        -m|--message) SUMMARY=${2:?-m needs a summary}; shift ;;
        --dry-run) DRY=1 ;;
        --allow-unsigned) UNSIGNED_OK=1 ;;
        -h|--help) sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "release: unknown option $1 (see --help)" >&2; exit 2 ;;
        *) [ -z "$VERSION" ] || { echo "release: one version, please" >&2; exit 2; }; VERSION=${1#v} ;;
    esac
    shift
done
[ -n "$VERSION" ] || { echo "usage: tools/release.sh <version> [-m summary] [--dry-run] [--allow-unsigned]" >&2; exit 2; }

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then B=$'\e[1m' R=$'\e[31m' G=$'\e[32m' Y=$'\e[33m' D=$'\e[2m' N=$'\e[0m'
else B="" R="" G="" Y="" D="" N=""; fi
step() { printf '\n%s== %s%s\n' "$B" "$*" "$N"; }
ok() { printf '  %s✓%s %s\n' "$G" "$N" "$*"; }
warn() { printf '  %s!%s %s\n' "$Y" "$N" "$*"; }
refuse() {
    printf '\n%s✗ release %s refused:%s %s\n' "$R$B" "$VERSION" "$N" "$1" >&2
    shift
    for l in "$@"; do printf '    %s\n' "$l" >&2; done
    exit 1
}

# ── versions ─────────────────────────────────────────────────────────
case "$VERSION" in
    *[!0-9A-Za-z.-]*) refuse "'$VERSION' has characters a version does not." ;;
esac
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$' ||
    refuse "'$VERSION' is not a version like 1.0.1 or 1.1.0-rc.1."
PRE=""; case "$VERSION" in *-*) PRE=1 ;; esac
CORE=${VERSION%%-*}
MAJOR=${CORE%%.*} REST=${CORE#*.}; MINOR=${REST%%.*} PATCH=${REST#*.}

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

CURRENT=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
LAST_TAG=$(git tag --list 'v[0-9]*' | while read -r t; do echo "$t"; done |
    { best=""; while read -r t; do if [ -z "$best" ] || semver_gt "$t" "$best"; then best=$t; fi; done; echo "$best"; })

printf '%sReleasing olang %s%s %s(Cargo.toml has %s; the last tag is %s)%s\n' "$B" "$VERSION" "$N" "$D" "$CURRENT" "${LAST_TAG:-none}" "$N"
[ $DRY -eq 1 ] && printf '%sdry run: nothing will be committed or tagged%s\n' "$Y" "$N"

# ── 1. pre-flight ────────────────────────────────────────────────────
step "1. Checks before anything changes"

[ "$(git rev-parse --abbrev-ref HEAD)" = "$BRANCH" ] ||
    refuse "you are on '$(git rev-parse --abbrev-ref HEAD)', not $BRANCH." "Releases are cut from $BRANCH: git switch $BRANCH"
ok "on $BRANCH"

dirty=$(git status --porcelain)
[ -z "$dirty" ] || refuse "the working tree is not clean:" "$dirty" "Commit or remove these first; a release is exactly a commit."
ok "the working tree is clean"

if git fetch --quiet origin "$BRANCH" 2>/dev/null; then
    behind=$(git rev-list --count "HEAD..origin/$BRANCH")
    ahead=$(git rev-list --count "origin/$BRANCH..HEAD")
    [ "$behind" -eq 0 ] || refuse "$BRANCH is $behind commit(s) behind origin/$BRANCH." "git pull --ff-only, then run this again."
    if [ "$ahead" -gt 0 ]; then warn "$ahead commit(s) not on origin yet; pushing $BRANCH publishes them with the release"
    else ok "level with origin/$BRANCH"; fi
else
    warn "could not fetch origin/$BRANCH (offline?): not checked against the remote"
fi

git rev-parse --verify --quiet "refs/tags/v$VERSION" >/dev/null && refuse "the tag v$VERSION exists already." "A released version is never re-tagged; release the next one."
if remote=$(git ls-remote --tags origin "refs/tags/v$VERSION" 2>/dev/null); then
    [ -z "$remote" ] || refuse "origin has the tag v$VERSION already."
fi
semver_gt "$VERSION" "$CURRENT" || [ "$VERSION" = "$CURRENT" ] ||
    refuse "$VERSION is older than the version Cargo.toml has ($CURRENT)."
[ -z "$LAST_TAG" ] || semver_gt "$VERSION" "$LAST_TAG" || refuse "$VERSION is not newer than the last release, $LAST_TAG."
ok "v$VERSION is new and above ${LAST_TAG:-every earlier release}"

# the [Unreleased] section: from its heading to the next ## heading
unreleased=$(awk '/^## \[Unreleased\]/ { on = 1; next } on && /^## \[/ { exit } on' CHANGELOG.md)
[ -n "$(printf '%s' "$unreleased" | tr -d '[:space:]')" ] ||
    refuse "CHANGELOG.md's [Unreleased] section is empty." "Write what this release changes there first (Added, Changed, Fixed)."
ok "CHANGELOG.md has an [Unreleased] section ($(printf '%s\n' "$unreleased" | grep -c '^- ') entries)"

# the signing key: pinned in all four places and its secret set
# pem FILE NAME: the PEM a constant named NAME holds, however it is
# quoted (one line with \n escapes, or a multi-line string); empty when
# the constant is "" or ''.
pem() {
    awk -v name="$2" '
        !on && index($0, name) && /=/ {
            rest = substr($0, index($0, "=") + 1)
            if (rest ~ /^[[:space:]]*(""|\047\047)/) exit
            on = 1
        }
        on { buf = buf $0 "\n"; if (index($0, "-----END PUBLIC KEY-----")) exit }
        END {
            gsub(/\\n/, "\n", buf); gsub(/\r/, "", buf)
            b = index(buf, "-----BEGIN PUBLIC KEY-----"); e = index(buf, "-----END PUBLIC KEY-----")
            if (b && e > b) {
                body = substr(buf, b, e - b + length("-----END PUBLIC KEY-----"))
                n = split(body, ls, "\n")
                for (i = 1; i <= n; i++) { l = ls[i]; gsub(/^[[:space:]"\047@]+|[[:space:]"\047@;]+$/, "", l); if (l != "") print l }
            }
        }' "$1"
}
key_sh=$(pem install.sh 'SIGNING_KEY')
key_ps=$(pem install.ps1 '\$SigningKey')
key_rs=$(pem otc/src/commands/update.rs 'RELEASE_SIGNING_KEY: &str')
key_file=$( [ -f dist/signing/olang-release.pub ] && tr -d '\r' <dist/signing/olang-release.pub || true)
signed_from=$(sed -n 's/^SIGNED_FROM="\(.*\)"$/\1/p' install.sh)
if [ -z "$key_sh" ]; then
    if [ -n "$PRE" ] && [ $UNSIGNED_OK -eq 1 ]; then
        warn "no release signing key is pinned (OL-491): this release candidate goes out UNSIGNED, as --allow-unsigned asks"
    else
        hint=(); [ -z "$PRE" ] || hint=("A release candidate may go without it: --allow-unsigned.")
        refuse "no release signing key is pinned in install.sh (OL-491)." \
            "Make the key and pin it first: dist/signing/README.md, 'Setting it up (once)'." ${hint[@]+"${hint[@]}"}
    fi
else
    [ "$key_sh" = "$key_file" ] || refuse "dist/signing/olang-release.pub is missing or is not install.sh's SIGNING_KEY."
    [ "$key_sh" = "$key_ps" ] || refuse "install.ps1's \$SigningKey is not install.sh's SIGNING_KEY."
    [ "$key_sh" = "$key_rs" ] || refuse "otc's RELEASE_SIGNING_KEY (otc/src/commands/update.rs) is not install.sh's SIGNING_KEY."
    [ -n "$signed_from" ] || refuse "install.sh has a SIGNING_KEY but no SIGNED_FROM." "Set SIGNED_FROM (and \$SignedFrom, otc's SIGNED_FROM) to the first signed release."
    openssl pkey -pubin -noout -in <(printf '%s\n' "$key_sh") 2>/dev/null || refuse "install.sh's SIGNING_KEY is not a public key openssl can read."
    ok "the release signing key is pinned in install.sh, install.ps1, otc and dist/signing (signed from $signed_from)"
    if command -v gh >/dev/null && secrets=$(gh secret list --repo "$REPO" 2>/dev/null); then
        printf '%s\n' "$secrets" | grep -q '^OLANG_SIGNING_KEY' ||
            refuse "the OLANG_SIGNING_KEY secret is not set on $REPO, so CI cannot sign." "gh secret set OLANG_SIGNING_KEY --repo $REPO < olang-release.key"
        ok "the OLANG_SIGNING_KEY secret is set on $REPO"
    else
        warn "could not list $REPO's secrets (gh missing or not signed in): check OLANG_SIGNING_KEY is set before pushing the tag"
    fi
fi

# intended compatibility breaks for this version: reviewed, and said
breaks=$(awk -v v="$VERSION" '
    /^\[\[break\]\]/ { if (ver == v) print what "|" issue "|" kind; what = issue = kind = ver = ""; next }
    /^[[:space:]]*what[[:space:]]*=/ { what = $0; sub(/^[^"]*"/, "", what); sub(/".*/, "", what) }
    /^[[:space:]]*issue[[:space:]]*=/ { issue = $0; sub(/^[^"]*"/, "", issue); sub(/".*/, "", issue) }
    /^[[:space:]]*kind[[:space:]]*=/ { kind = $0; sub(/^[^"]*"/, "", kind); sub(/".*/, "", kind) }
    /^[[:space:]]*version[[:space:]]*=/ { ver = $0; sub(/^[^"]*"/, "", ver); sub(/".*/, "", ver) }
    END { if (ver == v) print what "|" issue "|" kind }' tests/compat/allow.toml)
if [ -z "$breaks" ]; then
    ok "no intended compatibility break is listed for $VERSION (tests/compat/allow.toml)"
else
    while IFS='|' read -r what issue kind; do
        case "$kind" in
            major) [ "$MINOR" = 0 ] && [ "$PATCH" = 0 ] || refuse "$what is a major break ($issue), and $VERSION is not a new major version." ;;
            experimental) [ "$PATCH" = 0 ] || refuse "$what changes an Experimental surface ($issue), which a patch release may not." ;;
        esac
        printf '%s\n' "$unreleased" | grep -q "$issue" ||
            refuse "tests/compat/allow.toml lists a $kind break of $what for $VERSION ($issue), and the CHANGELOG's [Unreleased] section does not mention $issue." \
                "Every intended break is said in the release notes."
        warn "INTENDED BREAK in $VERSION: $what ($kind, $issue), said in the CHANGELOG"
    done <<<"$breaks"
fi

# the gates CI runs, on this tree
TARGET=${CARGO_TARGET_DIR:-target}
if [ "${OLANG_RELEASE_BUILD:-}" = 1 ]; then
    BIN=${OLANG_RELEASE_BIN:-$TARGET/release/olang}
else
    BIN=${OLANG_RELEASE_BIN:-$TARGET/fasttest/olang}
fi
if [ "${OLANG_RELEASE_SKIP_GATES:-}" = 1 ]; then
    [ $DRY -eq 1 ] || refuse "OLANG_RELEASE_SKIP_GATES=1 is only for --dry-run: a release runs its gates."
    warn "OLANG_RELEASE_SKIP_GATES=1: fmt, clippy, the tests and the build were NOT run"
else
    gate() { local what=$1; shift; printf '  %s… %s%s\n' "$D" "$what" "$N"; "$@" >"$LOG" 2>&1 || { tail -40 "$LOG" >&2; refuse "$what failed (the whole log: $LOG)."; }; ok "$what"; }
    LOG=$(mktemp "${TMPDIR:-/tmp}/olang-release-gate.XXXXXX")
    gate "cargo fmt --check" $CARGO fmt --all -- --check
    gate "cargo clippy -D warnings" $CARGO clippy --workspace --all-targets -- -D warnings
    gate "the test suite (cargo test --workspace --profile fasttest)" $CARGO test --workspace --profile fasttest
    if [ "${OLANG_RELEASE_BUILD:-}" = 1 ]; then
        gate "a release build" $CARGO build --release --locked -p olang -p otc
    fi
    [ -x "$BIN" ] || refuse "no olang at $BIN (set OLANG_RELEASE_BIN)."
    gate "the examples (examples/run_all.ol)" sh -c "cd examples && '$(cd "$(dirname "$BIN")" && pwd)/olang' run run_all.ol"
fi
if [ -x "$BIN" ] || command -v olang >/dev/null; then
    tools/compat-check.sh $([ -x "$BIN" ] && echo --olang "$BIN") >"${LOG:=$(mktemp)}" 2>&1 ||
        { cat "$LOG" >&2; refuse "tools/compat-check.sh found a break of the 1.x promise." "Fix it, or record an intended break in tests/compat/allow.toml (RELEASING.md)."; }
    ok "tools/compat-check.sh: the 1.x promise holds"
else
    refuse "no olang binary to run tools/compat-check.sh with."
fi

# ── 2. the version bump ──────────────────────────────────────────────
step "2. Bump the version to $VERSION"
TODAY=$(date +%Y-%m-%d)
bumped="Cargo.toml playground/Cargo.toml Cargo.lock CHANGELOG.md"
put_back() { git checkout -- $bumped; [ -z "${SNAP:-}" ] || rm -f "$SNAP"; }
[ $DRY -eq 1 ] && trap put_back EXIT

# the first `version =` of each manifest is its package's
for m in Cargo.toml playground/Cargo.toml; do
    awk -v v="$VERSION" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' "$m" >"$m.new" && mv "$m.new" "$m"
done
# Cargo.lock: the entries of the two packages that carry the version
awk -v v="$VERSION" '
    /^name = "olang"$/ || /^name = "olang-playground"$/ { mark = 1; print; next }
    mark && /^version = "/ { print "version = \"" v "\""; mark = 0; next }
    { mark = 0; print }' Cargo.lock >Cargo.lock.new && mv Cargo.lock.new Cargo.lock
ok "Cargo.toml, playground/Cargo.toml and Cargo.lock say $VERSION"

# CHANGELOG: open the section under [Unreleased], and the compare links
PREV=${LAST_TAG:-}
awk -v v="$VERSION" -v d="$TODAY" -v prev="$PREV" -v repo="$REPO" '
    /^## \[Unreleased\]$/ && !h { print; print ""; print "## [" v "] - " d; h = 1; next }
    /^\[Unreleased\]: / && !l {
        print "[Unreleased]: https://github.com/" repo "/compare/v" v "...HEAD"
        if (prev != "") print "[" v "]: https://github.com/" repo "/compare/" prev "...v" v
        else print "[" v "]: https://github.com/" repo "/releases/tag/v" v
        l = 1; next }
    { print }' CHANGELOG.md >CHANGELOG.md.new && mv CHANGELOG.md.new CHANGELOG.md
ok "CHANGELOG.md: [$VERSION] - $TODAY opened, with its compare link"

# a new minor's stdlib surface, which every later 1.x is held to
SNAP=""
if [ -z "$PRE" ] && [ "$PATCH" = 0 ] && [ "$MAJOR" -ge 1 ] && [ ! -f "tests/compat/surface/$MAJOR.$MINOR.txt" ]; then
    SNAP="tests/compat/surface/$MAJOR.$MINOR.txt"
    if [ "${OLANG_RELEASE_SKIP_GATES:-}" = 1 ]; then
        warn "would write $SNAP (skipped with the gates)"; SNAP=""
    else
        OLANG_COMPAT_SNAPSHOT="$MAJOR.$MINOR" $CARGO test --profile fasttest --test compat_test compat_surface >"$LOG" 2>&1 ||
            { tail -30 "$LOG" >&2; refuse "writing $SNAP failed."; }
        [ -s "$SNAP" ] || refuse "the compat test ran but wrote no $SNAP."
        ok "wrote $SNAP ($(grep -vc '^#' "$SNAP") names)"
    fi
fi
[ "${OLANG_RELEASE_SKIP_GATES:-}" = 1 ] || {
    $CARGO metadata --locked --format-version 1 >/dev/null 2>"$LOG" || { cat "$LOG" >&2; refuse "Cargo.lock does not match the bumped manifests."; }
    ok "Cargo.lock agrees with the manifests"
}

git --no-pager diff --stat
if [ $DRY -eq 1 ]; then
    git --no-pager diff -U1 CHANGELOG.md | head -30
    step "Dry run done: every check passed. The tree is put back; nothing was committed or tagged."
    exit 0
fi

# ── 3. the commit and the tag, here only ─────────────────────────────
step "3. Commit and tag (locally)"
git add $bumped ${SNAP:+"$SNAP"}
git commit --quiet -m "$VERSION${SUMMARY:+: $SUMMARY}"
git tag -a "v$VERSION" -m "olang $VERSION"
ok "committed $(git rev-parse --short HEAD) \"$VERSION${SUMMARY:+: $SUMMARY}\" and tagged v$VERSION (not pushed)"

cat <<EOF

${B}Next, by hand (RELEASING.md, steps 4 to 8):${N}

  # look once more at what is about to go out
  git show --stat HEAD

  # publish: main first, then the tag, which starts the release workflow
  git push origin $BRANCH
  git push origin v$VERSION

  # watch it build, sign and publish (about 30 minutes)
  gh run watch \$(gh run list --repo $REPO --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId') --repo $REPO

  # then prove the published release: signature, checksums, a clean install, a smoke test
  tools/release-check.sh v$VERSION
EOF
[ -n "$PRE" ] || cat <<EOF

  # crates.io (olangc, olangc-ods, olangc-im): check, then publish
  tools/publish-crates.sh --dry-run
  tools/publish-crates.sh
EOF
cat <<EOF

To undo before pushing:  git tag -d v$VERSION && git reset --hard HEAD~1
EOF
