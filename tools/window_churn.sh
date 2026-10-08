#!/usr/bin/env bash
# window_churn.sh — run tools/window_churn.ol RUNS times (each opening and
# closing WINDOWS real windows) and count the runs a signal ended (a crash
# in a window's teardown). macOS: also counts the crash reports written.
#
#   tools/window_churn.sh [RUNS=50] [WINDOWS=3] [LINGER_MS=0]
#   OLANG=path/to/olang tools/window_churn.sh
set -u
cd "$(dirname "$0")/.."
OLANG="${OLANG:-target/release/olang}"
runs="${1:-50}"; windows="${2:-3}"; linger="${3:-0}"
reports() { ls "$HOME/Library/Logs/DiagnosticReports"/olang-*.ips 2>/dev/null | wc -l | tr -d ' '; }
before=$(reports)
ok=0; crashed=0; failed=0
for i in $(seq "$runs"); do
  "$OLANG" tools/window_churn.ol "$windows" "$linger" > /dev/null 2>&1
  s=$?
  if [ "$s" -eq 0 ]; then ok=$((ok + 1))
  elif [ "$s" -gt 128 ]; then crashed=$((crashed + 1)); echo "run $i: ended by signal $((s - 128))"
  else failed=$((failed + 1)); echo "run $i: exit $s"; fi
done
echo "window_churn: $runs runs of $windows windows — $ok ok, $crashed crashed, $failed failed; $(( $(reports) - before )) new crash reports"
[ "$crashed" -eq 0 ] && [ "$failed" -eq 0 ]
