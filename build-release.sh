#!/usr/bin/env bash
set -euo pipefail

NO_INSTALL_TOOLS=0
NO_PAUSE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-install-tools)
      NO_INSTALL_TOOLS=1
      ;;
    --no-pause)
      NO_PAUSE=1
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
  shift
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

REPRO_ROOT="$ROOT/target/reproducible-release"
WORKTREE_A="$REPRO_ROOT/source-a"
WORKTREE_B="$REPRO_ROOT/source-b"
TARGET_A="$REPRO_ROOT/target-a"
TARGET_B="$REPRO_ROOT/target-b"

pause_if_interactive() {
  local status="$1"
  if [[ "$NO_PAUSE" == "0" && -t 0 && -z "${CI:-}" ]]; then
    echo
    if [[ "$status" == "0" ]]; then
      read -r -p "Release build complete. Press Enter to close..." _ || true
    else
      read -r -p "Release build failed. Press Enter to close..." _ || true
    fi
  fi
}

remove_worktree() {
  local path="$1"
  git worktree remove --force "$path" >/dev/null 2>&1 || true
  if [[ -e "$path" ]]; then
    rm -rf "$path"
  fi
}

cleanup_worktrees() {
  set +e
  remove_worktree "$WORKTREE_A"
  remove_worktree "$WORKTREE_B"
  git worktree prune >/dev/null 2>&1 || true
  set -e
}

on_exit() {
  local status=$?
  trap - EXIT
  cleanup_worktrees
  pause_if_interactive "$status"
  exit "$status"
}
trap on_exit EXIT

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

assert_clean_worktree() {
  local status
  status="$(git status --porcelain=v1 --untracked-files=all)"
  if [[ -n "$status" ]]; then
    echo "$status" >&2
    fail "official release builds require a clean Git working tree"
  fi
}

join_encoded_rustflags() {
  local separator=$'\x1f'
  local joined=""
  local flag
  for flag in "$@"; do
    if [[ -n "$joined" ]]; then
      joined+="$separator"
    fi
    joined+="$flag"
  done
  printf '%s' "$joined"
}

build_once() {
  local source_root="$1"
  local target_dir="$2"
  local cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  local rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
  local -a rustflags=(
    "--remap-path-prefix=$source_root=/src/p2p-net"
    "--remap-path-prefix=$target_dir=/build/target"
    "--remap-path-prefix=$cargo_home=/cargo-home"
    "--remap-path-prefix=$rustup_home=/rustup-home"
    "-C"
    "linker=cc"
    "-C"
    "link-arg=-Wl,--build-id=sha1"
  )
  local encoded_rustflags
  encoded_rustflags="$(join_encoded_rustflags "${rustflags[@]}")"

  rm -rf "$target_dir"
  mkdir -p "$target_dir"

  (
    cd "$source_root"
    unset RUSTFLAGS RUSTC RUSTDOC RUSTDOCFLAGS RUSTC_BOOTSTRAP
    unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
    unset CARGO_BUILD_RUSTC CARGO_BUILD_RUSTC_WRAPPER CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER
    unset CARGO_BUILD_RUSTFLAGS CARGO_BUILD_TARGET CARGO_BUILD_TARGET_DIR
    unset CARGO_PROFILE_RELEASE_OPT_LEVEL CARGO_PROFILE_RELEASE_DEBUG
    unset CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS
    unset CARGO_PROFILE_RELEASE_LTO CARGO_PROFILE_RELEASE_PANIC
    unset CARGO_PROFILE_RELEASE_CODEGEN_UNITS CARGO_PROFILE_RELEASE_RPATH
    unset CARGO_PROFILE_RELEASE_STRIP CARGO_PROFILE_RELEASE_INCREMENTAL
    unset CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO
    unset CC CXX AR CFLAGS CPPFLAGS CXXFLAGS LDFLAGS
    export CARGO_ENCODED_RUSTFLAGS="$encoded_rustflags"
    export CARGO_TARGET_DIR="$target_dir"
    export CARGO_INCREMENTAL=0
    export CARGO_BUILD_JOBS=1
    export CARGO_TERM_COLOR=never
    export CARGO_NET_OFFLINE=true
    export SOURCE_DATE_EPOCH="$SOURCE_EPOCH"
    export TZ=UTC
    export LANG=C
    export LC_ALL=C
    cargo build --release --locked --offline --features dashboard --example p2p_node -j 1
  )
}

[[ "$(uname -s)" == "Linux" ]] || fail "build-release.sh is the canonical Linux release runner"

for command in git cargo rustc cc sha256sum cmp; do
  require_command "$command"
done

[[ -d .git ]] || fail "release runner must be executed from a Git checkout"

assert_clean_worktree
cleanup_worktrees
rm -rf "$REPRO_ROOT"

VALIDATION_ARGS=(--no-pause)
if [[ "$NO_INSTALL_TOOLS" == "1" ]]; then
  VALIDATION_ARGS+=(--no-install-tools)
fi

echo "p2p-net canonical Linux release build"
echo "Root: $ROOT"
echo
echo "==> Full production validation"
"$ROOT/run-full-validation.sh" "${VALIDATION_ARGS[@]}"

assert_clean_worktree

echo
echo "==> Resolve release identity"
COMMIT="$(git rev-parse HEAD)"
TREE="$(git rev-parse HEAD^{tree})"
SOURCE_EPOCH="$(git show -s --format=%ct HEAD)"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
RUSTC_VERSION="$(rustc --version)"
CARGO_VERSION="$(cargo --version)"

[[ -n "$HOST_TARGET" ]] || fail "could not determine rustc host target"
[[ "$HOST_TARGET" == *-unknown-linux-* ]] || fail "Linux release runner requires a Linux Rust host target (found: $HOST_TARGET)"
[[ -n "$VERSION" ]] || fail "could not determine package version from Cargo.toml"
[[ "$RUSTC_VERSION" == rustc\ 1.98.0\ * ]] || fail "release requires rustc 1.98.0 exactly (found: $RUSTC_VERSION)"

cargo metadata --locked --format-version 1 >/dev/null

echo "Commit: $COMMIT"
echo "Tree: $TREE"
echo "Target: $HOST_TARGET"
echo "SOURCE_DATE_EPOCH: $SOURCE_EPOCH"
echo "Rust: $RUSTC_VERSION"
echo "Cargo: $CARGO_VERSION"

echo
echo "==> Create independent clean source worktrees"
mkdir -p "$REPRO_ROOT"
git worktree add --detach "$WORKTREE_A" "$COMMIT" >/dev/null
git worktree add --detach "$WORKTREE_B" "$COMMIT" >/dev/null

echo
echo "==> Reproducibility build A"
build_once "$WORKTREE_A" "$TARGET_A"

echo
echo "==> Reproducibility build B"
build_once "$WORKTREE_B" "$TARGET_B"

ARTIFACT_A="$TARGET_A/release/examples/p2p_node"
ARTIFACT_B="$TARGET_B/release/examples/p2p_node"
[[ -f "$ARTIFACT_A" ]] || fail "release artifact A was not produced: $ARTIFACT_A"
[[ -f "$ARTIFACT_B" ]] || fail "release artifact B was not produced: $ARTIFACT_B"

HASH_A="$(sha256sum "$ARTIFACT_A" | awk '{print $1}')"
HASH_B="$(sha256sum "$ARTIFACT_B" | awk '{print $1}')"

echo
echo "==> Verify byte-for-byte reproducibility"
echo "Build A SHA-256: $HASH_A"
echo "Build B SHA-256: $HASH_B"
if [[ "$HASH_A" != "$HASH_B" ]] || ! cmp -s "$ARTIFACT_A" "$ARTIFACT_B"; then
  fail "independent clean release builds are not byte-for-byte identical; targets are retained under $REPRO_ROOT for investigation"
fi

echo "Reproducibility verified: both clean builds are byte-for-byte identical."

DIST_DIR="$ROOT/dist/$HOST_TARGET"
ARTIFACT_NAME="p2p_node-$VERSION-$HOST_TARGET"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"
cp "$ARTIFACT_A" "$DIST_DIR/$ARTIFACT_NAME"

printf '%s  %s\n' "$HASH_A" "$ARTIFACT_NAME" >"$DIST_DIR/SHA256SUMS.txt"
cat >"$DIST_DIR/BUILD-MANIFEST.txt" <<EOF_MANIFEST
project=p2p-net
version=$VERSION
git_commit=$COMMIT
git_tree=$TREE
source_date_epoch=$SOURCE_EPOCH
rustc=$RUSTC_VERSION
cargo=$CARGO_VERSION
target=$HOST_TARGET
linker=cc
profile=release
features=default,dashboard
artifact=$ARTIFACT_NAME
sha256=$HASH_A
reproducibility=verified-two-independent-clean-worktrees
validation=run-full-validation.sh-passed
EOF_MANIFEST

echo
echo "==> Release output"
echo "Artifact: $DIST_DIR/$ARTIFACT_NAME"
echo "Manifest: $DIST_DIR/BUILD-MANIFEST.txt"
echo "Checksums: $DIST_DIR/SHA256SUMS.txt"
echo
echo "Canonical Linux release build completed successfully."
