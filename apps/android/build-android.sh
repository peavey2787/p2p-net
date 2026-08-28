#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ANDROID_DIR="$ROOT_DIR/apps/android"

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

command -v rustc >/dev/null 2>&1 || fail "rustc is required"
command -v cargo >/dev/null 2>&1 || fail "cargo is required"
command -v rustup >/dev/null 2>&1 || fail "rustup is required"
command -v java >/dev/null 2>&1 || fail "JDK 17 is required"
command -v gradle >/dev/null 2>&1 || fail "Gradle 9.5.0 is required on PATH"

RUST_VERSION="$(rustc --version)"
[[ "$RUST_VERSION" == rustc\ 1.98.0\ * ]] || fail "expected rustc 1.98.0, got: $RUST_VERSION"

CARGO_NDK_VERSION="$(cargo ndk --version 2>/dev/null || true)"
[[ "$CARGO_NDK_VERSION" == *"4.1.2"* ]] || fail "cargo-ndk 4.1.2 is required (cargo install cargo-ndk --version 4.1.2 --locked)"

JAVA_VERSION="$(java -version 2>&1 | head -n 1)"
[[ "$JAVA_VERSION" =~ \"17([\._]|\") ]] || fail "Android builds require JDK 17, got: $JAVA_VERSION"

GRADLE_VERSION="$(gradle --version | awk '/^Gradle / {print $2; exit}')"
[[ "$GRADLE_VERSION" == "9.5.0" ]] || fail "expected Gradle 9.5.0, got: ${GRADLE_VERSION:-unknown}"

SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [[ -z "$SDK_ROOT" && -f "$ANDROID_DIR/local.properties" ]]; then
  SDK_ROOT="$(sed -n 's/^sdk\.dir=//p' "$ANDROID_DIR/local.properties" | head -n 1 | sed 's#\\:#:#g; s#\\\\#\\#g')"
fi
[[ -n "$SDK_ROOT" ]] || fail "set ANDROID_SDK_ROOT/ANDROID_HOME or create apps/android/local.properties with sdk.dir=..."
[[ -f "$SDK_ROOT/platforms/android-37.0/android.jar" ]] || fail "Android SDK platform 37.0 is missing under $SDK_ROOT"
[[ -d "$SDK_ROOT/ndk/28.2.13676358" ]] || fail "Android NDK 28.2.13676358 is missing under $SDK_ROOT"
[[ -x "$SDK_ROOT/cmake/3.22.1/bin/cmake" ]] || fail "Android SDK CMake 3.22.1 is missing under $SDK_ROOT"

# cargo-ndk does not read Gradle local.properties. Export the SDK/NDK paths
# resolved above so the native build uses the exact same pinned installation
# as AGP even when the developer intentionally keeps Android env vars unset.
export ANDROID_SDK_ROOT="$SDK_ROOT"
export ANDROID_HOME="$SDK_ROOT"
export ANDROID_NDK_HOME="$SDK_ROOT/ndk/28.2.13676358"

INSTALLED_TARGETS="$(rustup target list --installed --toolchain 1.98.0)"
grep -qx 'aarch64-linux-android' <<<"$INSTALLED_TARGETS" || fail "missing Rust target aarch64-linux-android (rustup target add aarch64-linux-android --toolchain 1.98.0)"
grep -qx 'x86_64-linux-android' <<<"$INSTALLED_TARGETS" || fail "missing Rust target x86_64-linux-android (rustup target add x86_64-linux-android --toolchain 1.98.0)"

(
  cd "$ROOT_DIR"
  cargo metadata --locked --format-version 1 >/dev/null
)

SOURCE_DATE_EPOCH="$(git -C "$ROOT_DIR" log -1 --format=%ct 2>/dev/null || true)"
[[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]] || SOURCE_DATE_EPOCH=1577836800
export SOURCE_DATE_EPOCH CARGO_INCREMENTAL=0

EVIDENCE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/p2p-net-android-repro.XXXXXX")"
cleanup() { rm -rf "$EVIDENCE_ROOT"; }
trap cleanup EXIT

remove_android_build_state() {
  rm -rf \
    "$ANDROID_DIR/app/build" \
    "$ANDROID_DIR/app/.cxx" \
    "$ANDROID_DIR/app/src/main/jniLibs"
}

build_once() {
  local label="$1"
  local build_root="$EVIDENCE_ROOT/$label"
  local cargo_target="$build_root/cargo-target"
  local artifact_root="$build_root/artifacts"
  mkdir -p "$cargo_target" "$artifact_root"
  remove_android_build_state
  export CARGO_TARGET_DIR="$cargo_target"

  printf '\n==> Android reproducibility build %s\n' "$label"
  (
    cd "$ANDROID_DIR"
    gradle --no-daemon --no-build-cache --no-configuration-cache --rerun-tasks --stacktrace \
      clean :app:assembleRelease :app:bundleRelease
  )

  local apk="$ANDROID_DIR/app/build/outputs/apk/release/app-release-unsigned.apk"
  local aab="$ANDROID_DIR/app/build/outputs/bundle/release/app-release.aab"
  [[ -f "$apk" ]] || fail "Android release build $label did not produce $apk"
  [[ -f "$aab" ]] || fail "Android release build $label did not produce $aab"
  cp "$apk" "$artifact_root/app-release-unsigned.apk"
  cp "$aab" "$artifact_root/app-release.aab"
}

build_once build-a
build_once build-b

APK_A="$EVIDENCE_ROOT/build-a/artifacts/app-release-unsigned.apk"
APK_B="$EVIDENCE_ROOT/build-b/artifacts/app-release-unsigned.apk"
AAB_A="$EVIDENCE_ROOT/build-a/artifacts/app-release.aab"
AAB_B="$EVIDENCE_ROOT/build-b/artifacts/app-release.aab"
APK_HASH_A="$(sha256sum "$APK_A" | awk '{print $1}')"
APK_HASH_B="$(sha256sum "$APK_B" | awk '{print $1}')"
AAB_HASH_A="$(sha256sum "$AAB_A" | awk '{print $1}')"
AAB_HASH_B="$(sha256sum "$AAB_B" | awk '{print $1}')"

printf '\n==> Verify Android byte-for-byte reproducibility\n'
printf 'APK build A SHA-256: %s\nAPK build B SHA-256: %s\n' "$APK_HASH_A" "$APK_HASH_B"
printf 'AAB build A SHA-256: %s\nAAB build B SHA-256: %s\n' "$AAB_HASH_A" "$AAB_HASH_B"

DIST_DIR="$ROOT_DIR/dist/android"
APK_BYTE_IDENTICAL=false
AAB_BYTE_IDENTICAL=false
if cmp -s "$APK_A" "$APK_B"; then APK_BYTE_IDENTICAL=true; fi
if cmp -s "$AAB_A" "$AAB_B"; then AAB_BYTE_IDENTICAL=true; fi
printf 'APK direct byte comparison: %s\nAAB direct byte comparison: %s\n' "$APK_BYTE_IDENTICAL" "$AAB_BYTE_IDENTICAL"

if [[ "$APK_HASH_A" != "$APK_HASH_B" || "$AAB_HASH_A" != "$AAB_HASH_B" || "$APK_BYTE_IDENTICAL" != true || "$AAB_BYTE_IDENTICAL" != true ]]; then
  FAILURE_DIR="$DIST_DIR/reproducibility-failure"
  mkdir -p "$FAILURE_DIR"
  cp "$APK_A" "$FAILURE_DIR/build-a.apk"
  cp "$APK_B" "$FAILURE_DIR/build-b.apk"
  cp "$AAB_A" "$FAILURE_DIR/build-a.aab"
  cp "$AAB_B" "$FAILURE_DIR/build-b.aab"
  fail "Android release is not reproducible; evidence retained at $FAILURE_DIR"
fi

mkdir -p "$DIST_DIR"
DIST_APK="$DIST_DIR/p2p-net-android-0.1.0-release-unsigned.apk"
DIST_AAB="$DIST_DIR/p2p-net-android-0.1.0-release.aab"
cp "$APK_B" "$DIST_APK"
cp "$AAB_B" "$DIST_AAB"
printf '%s  %s\n%s  %s\n' \
  "$APK_HASH_B" "$(basename "$DIST_APK")" \
  "$AAB_HASH_B" "$(basename "$DIST_AAB")" > "$DIST_DIR/SHA256SUMS.txt"
cat > "$DIST_DIR/BUILD-MANIFEST.txt" <<EOF
p2p-net Android reproducible release
version=0.1.0
rust=1.98.0
cargo_ndk=4.1.2
gradle=9.5.0
jdk=17
android_sdk=37.0
ndk=28.2.13676358
cmake=3.22.1
abis=arm64-v8a,x86_64
source_date_epoch=$SOURCE_DATE_EPOCH
build_a_apk_sha256=$APK_HASH_A
build_b_apk_sha256=$APK_HASH_B
build_a_aab_sha256=$AAB_HASH_A
build_b_aab_sha256=$AAB_HASH_B
apk_byte_identical=$APK_BYTE_IDENTICAL
aab_byte_identical=$AAB_BYTE_IDENTICAL
reproducible=true
EOF

printf 'Reproducibility verified: both clean Android builds are byte-for-byte identical.\n'
printf '\nVerified unsigned APK: %s\n' "$DIST_APK"
printf 'Verified unsigned AAB: %s\n' "$DIST_AAB"
printf 'Checksums: %s\n' "$DIST_DIR/SHA256SUMS.txt"
printf 'Manifest: %s\n' "$DIST_DIR/BUILD-MANIFEST.txt"
