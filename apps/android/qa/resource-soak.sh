#!/usr/bin/env bash
set -euo pipefail
DURATION_MINUTES="${1:-60}"
SAMPLE_SECONDS="${2:-30}"
PACKAGE="${3:-io.github.peavey2787.p2pnet}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
[[ "$DURATION_MINUTES" =~ ^[0-9]+$ ]] && (( DURATION_MINUTES >= 1 )) || { echo "invalid duration" >&2; exit 2; }
[[ "$SAMPLE_SECONDS" =~ ^[0-9]+$ ]] && (( SAMPLE_SECONDS >= 5 )) || { echo "invalid sample interval" >&2; exit 2; }
command -v adb >/dev/null 2>&1 || { echo "adb is required" >&2; exit 1; }
[[ "$(adb get-state 2>/dev/null)" == "device" ]] || { echo "adb device is not ready" >&2; exit 1; }

OUT="$REPO_ROOT/apps/android/qa/evidence/resource-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
echo 'timestamp,pid,total_pss_kb,total_rss_kb,threads,fds,cpu_percent' > "$OUT/samples.csv"
DEADLINE=$(( $(date +%s) + DURATION_MINUTES * 60 ))
SAMPLE=0
while (( $(date +%s) < DEADLINE )); do
  PID="$(adb shell pidof "$PACKAGE" | tr -d '\r' | awk '{print $1}')"
  [[ -n "$PID" ]] || { echo "$PACKAGE is not running at sample $SAMPLE" >&2; exit 1; }
  MEM="$(adb shell dumpsys meminfo "$PACKAGE" | tr -d '\r')"
  STATUS="$(adb shell cat "/proc/$PID/status" | tr -d '\r')"
  CPU="$(adb shell dumpsys cpuinfo | tr -d '\r')"
  FDS="$(adb shell "ls /proc/$PID/fd 2>/dev/null | wc -l" 2>/dev/null | tr -d '\r' || true)"
  PSS="$(printf '%s\n' "$MEM" | sed -n 's/.*TOTAL PSS:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
  RSS="$(printf '%s\n' "$MEM" | sed -n 's/.*TOTAL RSS:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
  THREADS="$(printf '%s\n' "$STATUS" | sed -n 's/^Threads:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
  CPU_PERCENT="$(printf '%s\n' "$CPU" | awk -v p="$PACKAGE" '$0 ~ p {gsub(/^ +/, ""); sub(/%.*/, "", $0); print $0; exit}')"
  printf '%s,%s,%s,%s,%s,%s,%s\n' "$(date -Iseconds)" "$PID" "$PSS" "$RSS" "$THREADS" "$FDS" "$CPU_PERCENT" >> "$OUT/samples.csv"
  printf '%s\n' "$MEM" > "$OUT/meminfo-$(printf '%05d' "$SAMPLE").txt"
  SAMPLE=$((SAMPLE + 1))
  sleep "$SAMPLE_SECONDS"
done
[[ -n "$(adb shell pidof "$PACKAGE" | tr -d '\r')" ]] || { echo "$PACKAGE exited before the soak completed" >&2; exit 1; }
echo "Resource soak completed: $SAMPLE samples"
echo "Evidence: $OUT"
