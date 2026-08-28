#!/usr/bin/env bash
set -euo pipefail
CYCLES="${1:-100}"
SETTLE_SECONDS="${2:-3}"
PACKAGE="${3:-io.github.peavey2787.p2pnet}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
COMPONENT="$PACKAGE/.MainActivity"
[[ "$CYCLES" =~ ^[0-9]+$ ]] && (( CYCLES >= 1 )) || { echo "invalid cycle count" >&2; exit 2; }
command -v adb >/dev/null 2>&1 || { echo "adb is required" >&2; exit 1; }
OUT="$REPO_ROOT/apps/android/qa/evidence/lifecycle-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
echo 'cycle,pid,total_pss_kb,total_rss_kb,threads,fds' > "$OUT/cycles.csv"
for ((cycle=1; cycle<=CYCLES; cycle++)); do
  adb shell am force-stop "$PACKAGE" >/dev/null
  adb shell am start -W -n "$COMPONENT" >/dev/null
  sleep "$SETTLE_SECONDS"
  PID="$(adb shell pidof "$PACKAGE" | tr -d '\r' | awk '{print $1}')"
  [[ -n "$PID" ]] || { echo "$PACKAGE failed to return at cycle $cycle" >&2; exit 1; }
  MEM="$(adb shell dumpsys meminfo "$PACKAGE" | tr -d '\r')"
  STATUS="$(adb shell cat "/proc/$PID/status" | tr -d '\r')"
  PSS="$(printf '%s\n' "$MEM" | sed -n 's/.*TOTAL PSS:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
  RSS="$(printf '%s\n' "$MEM" | sed -n 's/.*TOTAL RSS:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
  THREADS="$(printf '%s\n' "$STATUS" | sed -n 's/^Threads:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
  FDS="$(adb shell "ls /proc/$PID/fd 2>/dev/null | wc -l" 2>/dev/null | tr -d '\r' || true)"
  printf '%s,%s,%s,%s,%s,%s\n' "$cycle" "$PID" "$PSS" "$RSS" "$THREADS" "$FDS" >> "$OUT/cycles.csv"
done
echo "Lifecycle cycles completed: $CYCLES"
echo "Evidence: $OUT"
