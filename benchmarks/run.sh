#!/bin/bash
# run.sh — the DP4 pipeline benchmark runner.
#
#   benchmarks/run.sh [reps] [rows]
#
# Requires: an `olang` binary on PATH (or OLANG=/path/to/olang) and a
# Python with pandas and polars installed (or PYTHON=/path/to/python).
# Generates the dataset if missing, runs each engine `reps` times
# (default 5), refuses the result if any checksum disagrees across
# engines or reps, and prints the per-stage median table.
set -euo pipefail
cd "$(dirname "$0")/.."

REPS="${1:-5}"
ROWS="${2:-1000000}"
OLANG="${OLANG:-olang}"
PYTHON="${PYTHON:-python3}"

if [ ! -f benchmarks/data/sales.csv ] || [ "$(wc -l < benchmarks/data/sales.csv)" -ne $((ROWS + 1)) ]; then
    echo "generating ${ROWS} rows..."
    "$OLANG" run benchmarks/gen_data.ol "$ROWS"
fi

run_engine() { # name, command...
    local name="$1"; shift
    for rep in $(seq 1 "$REPS"); do
        "$@" | while read -r kind rest; do
            echo "$name $rep $kind $rest"
        done
    done
}

RAW="$(mktemp)"
trap 'rm -f "$RAW"' EXIT
{
    run_engine ods "$OLANG" run benchmarks/pipeline.ol
    run_engine pandas "$PYTHON" benchmarks/pipeline_pandas.py
    run_engine polars "$PYTHON" benchmarks/pipeline_polars.py
} > "$RAW"

"$PYTHON" - "$RAW" "$REPS" <<'PY'
import statistics
import sys

raw, reps = sys.argv[1], int(sys.argv[2])
times = {}      # (engine, stage) -> [ms]
checks = {}     # stage -> {engine: {checksum}}
totals = {}     # engine -> [ms]
stages = []
for line in open(raw):
    parts = line.split()
    engine, rep, kind = parts[0], parts[1], parts[2]
    if kind == "TOTAL":
        totals.setdefault(engine, []).append(int(parts[3]))
        continue
    _, name, ms, checksum = parts[2], parts[3], parts[4], " ".join(parts[5:])
    if name not in stages:
        stages.append(name)
    times.setdefault((engine, name), []).append(int(ms))
    checks.setdefault(name, {}).setdefault(engine, set()).add(checksum)

bad = False
for name, per in checks.items():
    values = {frozenset(v) for v in per.values()}
    flat = {c for v in per.values() for c in v}
    if len(flat) != 1:
        print(f"CHECKSUM MISMATCH at stage '{name}': {per}", file=sys.stderr)
        bad = True
if bad:
    sys.exit(1)

engines = ["ods", "pandas", "polars"]
print(f"\nmedians of {reps} runs (ms)")
print(f"{'stage':<8}" + "".join(f"{e:>9}" for e in engines))
for name in stages:
    row = "".join(f"{statistics.median(times[(e, name)]):>9.0f}" for e in engines)
    print(f"{name:<8}{row}")
print(f"{'TOTAL':<8}" + "".join(f"{statistics.median(totals[e]):>9.0f}" for e in engines))
print("\nall checksums agree across engines and runs")
PY
