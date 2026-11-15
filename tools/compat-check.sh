#!/bin/bash
# tools/compat-check.sh — does this tree keep the 1.x promise?
#
#   tools/compat-check.sh [--against VERSION] [--olang BIN]
#
# The release-time half of the compatibility promise (docs/stability.md,
# tests/compat/README.md). tests/compat_test.rs is the other half and runs with the
# test suite. This script checks, against VERSION (default 1.0.0):
#
#   1. `olang check --against VERSION` over olang's own public packages
#      (PACKAGES below): no @stable item removed or re-signed.
#   2. Every compatibility program in tests/compat/programs/ prints what
#      its .out says and exits as its header says, run by BIN.
#   3. No expectation was changed silently: each .out, snapshot or
#      fixture that differs from tag vVERSION is named in
#      tests/compat/allow.toml, and no surface snapshot was edited.
#
# BIN defaults to ./.bin/olang, then target/release/olang, then `olang` on
# PATH. Before tag vVERSION exists (the 1.0 release candidates, and 1.0.0
# itself) 1 and 3 have nothing to compare with and say so; 2 still runs.
# Exit 0 when the promise holds, 1 on a break, 2 when it cannot check.
set -u
cd "$(dirname "$0")/.."

AGAINST=1.0.0
OLANG=""
while [ $# -gt 0 ]; do
    case "$1" in
        --against) AGAINST=${2:?--against needs a version}; shift ;;
        --olang) OLANG=${2:?--olang needs a binary}; shift ;;
        -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "compat-check: unknown option $1" >&2; exit 2 ;;
    esac
    shift
done
AGAINST=${AGAINST#v}

# olang's own packages with a public surface. A package listed here is
# held to the @stable items it had at $AGAINST.
PACKAGES="tests/compat/package frameworks/web-sdk examples/language/packages/geometry"

if [ -z "$OLANG" ]; then
    for c in ./.bin/olang ./target/release/olang; do [ -x "$c" ] && { OLANG=$c; break; }; done
    [ -n "$OLANG" ] || OLANG=$(command -v olang || true)
fi
[ -n "$OLANG" ] && [ -x "$OLANG" ] || { echo "compat-check: no olang binary (build one, or pass --olang BIN)" >&2; exit 2; }
OLANG=$(cd "$(dirname "$OLANG")" && pwd)/$(basename "$OLANG")

failures=0
fail() { echo "  ✗ $*"; failures=$((failures + 1)); }
ok() { echo "  ✓ $*"; }

echo "compat-check: $("$OLANG" --version) against $AGAINST"

TAG=""
for t in "v$AGAINST" "$AGAINST"; do
    git rev-parse --verify --quiet "refs/tags/$t^{commit}" >/dev/null && { TAG=$t; break; }
done

# ── 1. the public packages' @stable items ────────────────────────────
echo "1. @stable items of olang's packages, against $AGAINST"
if [ -z "$TAG" ]; then
    echo "  - no tag v$AGAINST yet: nothing to compare with (this is the release that sets it)"
else
    for pkg in $PACKAGES; do
        if [ ! -f "$pkg/olang.toml" ]; then fail "$pkg: no olang.toml"; continue; fi
        if ! git cat-file -e "$TAG:$pkg/olang.toml" 2>/dev/null; then
            echo "  - $pkg: not a package at $TAG (new since): nothing to hold it to"
            continue
        fi
        out=$(cd "$pkg" && "$OLANG" check --against "$AGAINST" . 2>&1)
        code=$?
        if [ $code -eq 0 ]; then ok "$pkg: $(printf '%s\n' "$out" | grep 'still stand' | sed 's/^olang check: //')"
        else fail "$pkg (olang check --against $AGAINST exited $code):"; printf '%s\n' "$out" | sed 's/^/      /'; fi
    done
fi

# ── 2. the compatibility programs ───────────────────────────────────
echo "2. compatibility programs (tests/compat/programs), run by $OLANG"
# allowed: whether allow.toml has an entry with what = "$1"
allowed() { grep -Eq "^[[:space:]]*what[[:space:]]*=[[:space:]]*\"$1\"" tests/compat/allow.toml; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/olang-compat.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
for f in tests/compat/programs/*.ol tests/compat/programs/*/main.ol; do
    [ -f "$f" ] || continue
    case "$f" in
        */main.ol) dir=$(dirname "$f"); name="programs/$(basename "$dir")"; exp="$dir.out" ;;
        *) dir=$(dirname "$f"); name="programs/$(basename "$f")"; exp="${f%.ol}.out" ;;
    esac
    hdr=$(sed -n '1,/^[^/]/{/^\/\/ compat:/p;}' "$f")
    want_exit=$(printf '%s' "$hdr" | sed -n 's/.*exit \([0-9][0-9]*\).*/\1/p')
    want_exit=${want_exit:-0}
    sub=""; case "$hdr" in *" test"*|*":test"*) sub=test ;; esac
    [ -f "$exp" ] || { fail "$name: no $(basename "$exp")"; continue; }
    (cd "$dir" && NO_COLOR=1 "$OLANG" $sub "$(basename "$f")" >"$tmp/out" 2>"$tmp/err")
    code=$?
    if [ -n "$sub" ]; then
        # a test fixture: its lines in order among the runner's report
        awk 'NR==FNR { want[++n] = $0; next } i < n && $0 == want[i + 1] { i++ } END { exit !(i == n) }' "$exp" "$tmp/out"
        same=$?
    else
        cmp -s "$exp" "$tmp/out"; same=$?
    fi
    if [ $code -eq "$want_exit" ] && [ $same -eq 0 ]; then
        ok "$name"
    elif allowed "$name"; then
        echo "  ~ $name changed, as tests/compat/allow.toml allows"
    else
        fail "$name: exit $code (want $want_exit)$([ $same -eq 0 ] || echo ', stdout differs:')"
        [ $same -eq 0 ] || diff "$exp" "$tmp/out" | head -20 | sed 's/^/      /'
        [ $code -eq "$want_exit" ] || head -5 "$tmp/err" | sed 's/^/      /'
    fi
done

# ── 3. no silent change to an expectation ───────────────────────────
echo "3. expectations changed since $AGAINST, each named in allow.toml"
if [ -z "$TAG" ]; then
    echo "  - no tag v$AGAINST yet: nothing to compare with"
else
    changed=$(git diff --name-status "$TAG" -- tests/compat/programs tests/compat/surface)
    if [ -z "$changed" ]; then ok "none changed"; fi
    printf '%s\n' "$changed" | while read -r status path _; do
        [ -n "$status" ] || continue
        case "$status:$path" in
            A*) continue ;; # a new fixture or snapshot: additive
            *:tests/compat/surface/*) echo "  ✗ $path was edited ($status): a snapshot is written once; list a removed name in allow.toml as surface:<name>"; echo x >>"$tmp/bad" ;;
            *:tests/compat/programs/*)
                rel=${path#tests/compat/}
                case "$rel" in
                    programs/*/*) what="programs/$(printf '%s' "$rel" | cut -d/ -f2)" ;;
                    *.out) what="${rel%.out}"; [ -d "tests/compat/$what" ] || what="$what.ol" ;;
                    *) what=$rel ;;
                esac
                if allowed "$what"; then echo "  ~ $path ($status), allowed as $what"
                else echo "  ✗ $path changed ($status) with no allow.toml entry for $what"; echo x >>"$tmp/bad"; fi ;;
        esac
    done
    [ -f "$tmp/bad" ] && failures=$((failures + $(wc -l <"$tmp/bad")))
fi

if [ $failures -eq 0 ]; then
    echo "compat-check: the 1.x promise holds"
    exit 0
fi
echo "compat-check: $failures break$([ $failures -eq 1 ] || echo s) of the 1.x promise (tests/compat/README.md, \"An intended break\")"
exit 1
