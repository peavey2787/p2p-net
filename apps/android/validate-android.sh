#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ANDROID_DIR="$ROOT_DIR/apps/android"

# Production source/security gates first. Android-specific compilation then
# repeats the release build with Lint and JVM tests enabled.
"$ROOT_DIR/run-full-validation.sh" --no-clean
"$ANDROID_DIR/build-android.sh"
REPRO_MANIFEST="$ROOT_DIR/dist/android/BUILD-MANIFEST.txt"
[[ -f "$REPRO_MANIFEST" ]] || { echo "ERROR: Android reproducibility manifest missing: $REPRO_MANIFEST" >&2; exit 1; }
for required in reproducible=true apk_byte_identical=true aab_byte_identical=true; do
  grep -qx "$required" "$REPRO_MANIFEST" || { echo "ERROR: Android release manifest missing $required" >&2; exit 1; }
done
cd "$ANDROID_DIR"
gradle --no-daemon --stacktrace :app:lintRelease :app:testReleaseUnitTest
