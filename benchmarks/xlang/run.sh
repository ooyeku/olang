#!/usr/bin/env bash
# The cross-language benchmark sweep: every benchmark implements one
# algorithm, the same way, in nine languages, and reports a CHECK value
# (validated for agreement across languages) and a self-timed MS for
# its measured section. The runner compiles what needs compiling, runs
# each pair with a timeout, medians the reps, and prints one table.
#
#   benchmarks/xlang/run.sh [reps] [benchmarks...]
#
# Defaults: 3 reps (a run slower than 20 s gets 1), all benchmarks.
# A missing toolchain skips its column with a note; a run past the
# timeout reports DNF. Protocol notes live in README.md — most
# importantly: plain loops and the language's natural data structures,
# no vectorization libraries, so the comparison measures the language,
# not its bindings.
set -uo pipefail
cd "$(dirname "$0")"

REPS="${1:-3}"
shift 2>/dev/null || true
BENCHES=("$@")
if [ ${#BENCHES[@]} -eq 0 ]; then
    BENCHES=(fib collatz sieve nbody matmul strbuild wordfreq kmeans)
fi
LANGS=(olang cpp rust go java js lua python R)
TIMEOUT="${XLANG_TIMEOUT:-120}"
SLOW_MS=20000
OLANG="${OLANG:-olang}"

mkdir -p build results

have() { command -v "$1" >/dev/null 2>&1; }

# Title-case for the Java class file (Fib.java, Collatz.java, ...).
title() { printf "%s" "$(tr '[:lower:]' '[:upper:]' <<<"${1:0:1}")${1:1}"; }

compile() { # bench lang -> 0 ok / 1 skip
    local b="$1" l="$2" t
    case "$l" in
        cpp)  [ build/"$b"_cpp -nt src/"$b".cpp ] 2>/dev/null && return 0
              have c++ || return 1
              c++ -O3 -ffp-contract=off -std=c++17 -o build/"$b"_cpp src/"$b".cpp ;;
        rust) [ build/"$b"_rs -nt src/"$b".rs ] 2>/dev/null && return 0
              have rustc || return 1
              rustc -O -o build/"$b"_rs src/"$b".rs 2>/dev/null ;;
        go)   [ build/"$b"_go -nt src/"$b".go ] 2>/dev/null && return 0
              have go || return 1
              go build -o build/"$b"_go src/"$b".go ;;
        java) t="$(title "$b")"
              [ build/java_"$b"/"$t".class -nt src/"$t".java ] 2>/dev/null && return 0
              have javac || return 1
              mkdir -p build/java_"$b"
              javac -d build/java_"$b" src/"$t".java ;;
        olang)  have "$OLANG" ;;
        js)     have node ;;
        lua)    have lua ;;
        python) have python3 ;;
        R)      have Rscript ;;
    esac
}

run_one() { # bench lang -> prints "CHECK MS" or "DNF" / "ERR"
    local b="$1" l="$2" t out
    case "$l" in
        olang)  out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" "$OLANG" run src/"$b".ol 2>/dev/null) ;;
        cpp)    out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" ./build/"$b"_cpp 2>/dev/null) ;;
        rust)   out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" ./build/"$b"_rs 2>/dev/null) ;;
        go)     out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" ./build/"$b"_go 2>/dev/null) ;;
        java)   out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" java -cp build/java_"$b" "$(title "$b")" 2>/dev/null) ;;
        js)     out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" node src/"$b".js 2>/dev/null) ;;
        lua)    out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" lua src/"$b".lua 2>/dev/null) ;;
        python) out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" python3 src/"$b".py 2>/dev/null) ;;
        R)      out=$(perl -e 'alarm shift; exec @ARGV' "$TIMEOUT" Rscript --vanilla src/"$b".R 2>/dev/null) ;;
    esac
    local rc=$?
    if [ $rc -eq 142 ] || [ $rc -eq 137 ]; then echo "DNF"; return; fi
    local check ms
    check=$(grep '^CHECK' <<<"$out" | awk '{print $2}')
    ms=$(grep '^MS' <<<"$out" | awk '{print $2}')
    if [ -z "$check" ] || [ -z "$ms" ]; then echo "ERR"; return; fi
    echo "$check $ms"
}

median() { sort -n | awk '{ a[NR]=$1 } END { print a[int((NR+1)/2)] }'; }

: > results/table.tsv
for b in "${BENCHES[@]}"; do
    echo "== $b =="
    CHECKS_SEEN=""
    for l in "${LANGS[@]}"; do
        if ! compile "$b" "$l"; then
            printf "  %-7s %s\n" "$l" "skipped (toolchain missing)"
            echo -e "$b\t$l\tSKIP" >> results/table.tsv
            continue
        fi
        first=$(run_one "$b" "$l")
        if [ "$first" = "DNF" ] || [ "$first" = "ERR" ]; then
            printf "  %-7s %s\n" "$l" "$first"
            echo -e "$b\t$l\t$first" >> results/table.tsv
            continue
        fi
        check=${first% *}
        ms=${first#* }
        CHECKS_SEEN="${CHECKS_SEEN}${l} ${check}
"
        times=("$ms")
        if [ "$ms" -lt "$SLOW_MS" ]; then
            for _ in $(seq 2 "$REPS"); do
                r=$(run_one "$b" "$l")
                [ "$r" = "DNF" ] || [ "$r" = "ERR" ] && continue
                times+=("${r#* }")
            done
        fi
        med=$(printf "%s\n" "${times[@]}" | median)
        printf "  %-7s %8s ms   (check %s, %d rep%s)\n" "$l" "$med" "$check" "${#times[@]}" "$([ ${#times[@]} -eq 1 ] && echo "" || echo "s")"
        echo -e "$b\t$l\t$med" >> results/table.tsv
    done
    # Checksum agreement across every language that finished.
    ref=""
    refl=""
    while read -r cl cv; do
        [ -z "$cl" ] && continue
        if [ -z "$ref" ]; then ref="$cv"; refl="$cl"; continue; fi
        if [ "$cv" != "$ref" ]; then
            echo "  !! CHECK DISAGREES: $cl=$cv vs $refl=$ref"
            echo -e "$b\tCHECKFAIL\t$cl" >> results/table.tsv
        fi
    done <<EOF_CHECKS
$CHECKS_SEEN
EOF_CHECKS
done

echo
echo "final medians (ms) — results/table.tsv"
python3 - <<'PYEOF'
rows = {}
langs = ["olang","cpp","rust","go","java","js","lua","python","R"]
benches = []
for line in open("results/table.tsv"):
    b, l, v = line.strip().split("\t")
    if l == "CHECKFAIL":
        continue
    if b not in rows:
        rows[b] = {}
        benches.append(b)
    rows[b][l] = v
print(f"{'bench':<10}" + "".join(f"{l:>9}" for l in langs))
for b in benches:
    print(f"{b:<10}" + "".join(f"{rows[b].get(l,'-'):>9}" for l in langs))
PYEOF
