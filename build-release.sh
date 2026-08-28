#!/usr/bin/env bash
set -euo pipefail

NO_INSTALL_TOOLS=0
NO_PAUSE=0
FORCE_VALIDATION=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-install-tools)
      NO_INSTALL_TOOLS=1
      ;;
    --no-pause)
      NO_PAUSE=1
      ;;
    --force-validation)
      FORCE_VALIDATION=1
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
ROOT_PARENT="$(dirname "$ROOT")"
ROOT_NAME="$(basename "$ROOT")"
WORKTREE_ROOT="$ROOT_PARENT/.${ROOT_NAME}-release-worktrees"
VALIDATION_WORKTREE="$WORKTREE_ROOT/validation-source"
WORKTREE_A="$WORKTREE_ROOT/source-a"
WORKTREE_B="$WORKTREE_ROOT/source-b"
TARGET_A="$REPRO_ROOT/target-a"
TARGET_B="$REPRO_ROOT/target-b"
RELEASE_INPUTS_FILE="$REPRO_ROOT/release-inputs.txt"

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
  remove_worktree "$VALIDATION_WORKTREE"
  remove_worktree "$WORKTREE_A"
  remove_worktree "$WORKTREE_B"
  git worktree prune >/dev/null 2>&1 || true
  rm -rf "$WORKTREE_ROOT"
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

capture_source_snapshot() {
  local index_file="$REPRO_ROOT/release-index"
  rm -f "$index_file"
  mkdir -p "$REPRO_ROOT"
  GIT_INDEX_FILE="$index_file" git read-tree HEAD
  GIT_INDEX_FILE="$index_file" git add -A
  SOURCE_TREE="$(GIT_INDEX_FILE="$index_file" git write-tree)"
  rm -f "$index_file"

  git ls-tree -r --full-tree "$SOURCE_TREE" -- \
    Cargo.toml Cargo.lock rust-toolchain.toml .cargo/config.toml crates apps external examples assets \
    >"$RELEASE_INPUTS_FILE"
  RELEASE_INPUT_SHA256="$(sha256sum "$RELEASE_INPUTS_FILE" | awk '{print $1}')"
  RELEASE_INPUT_COUNT="$(wc -l <"$RELEASE_INPUTS_FILE" | tr -d ' ')"
}

manifest_value() {
  local manifest="$1"
  local key="$2"
  sed -n "s/^${key}=//p" "$manifest" | head -n 1
}

find_validation_evidence() {
  local expected="$1"
  local search_root="${2:-$ROOT}"
  local manifest
  VALIDATION_EVIDENCE=""
  VALIDATION_EVIDENCE_KIND=""
  while IFS= read -r manifest; do
    [[ -f "$manifest" ]] || continue
    if [[ "$(manifest_value "$manifest" result)" == "pass" \
      && "$(manifest_value "$manifest" mode)" == "full" \
      && "$(manifest_value "$manifest" release_input_sha256)" == "$expected" ]]; then
      VALIDATION_EVIDENCE="$manifest"
      VALIDATION_EVIDENCE_KIND="$(manifest_value "$manifest" evidence_kind)"
      return 0
    fi
  done < <(
    find "$search_root/qa/evidence/runs" "$search_root/qa/evidence/recovered" "$search_root/qa/evidence/attestations" \
      -type f -name manifest.txt -printf '%T@ %p\n' 2>/dev/null \
      | sort -nr | cut -d' ' -f2-
  )
  return 1
}

create_snapshot_commit() {
  SNAPSHOT_COMMIT="$(
    printf 'p2p-net reproducible release snapshot\n' | env \
      GIT_AUTHOR_NAME='p2p-net release snapshot' \
      GIT_AUTHOR_EMAIL='release-snapshot@invalid' \
      GIT_AUTHOR_DATE="@$SOURCE_EPOCH +0000" \
      GIT_COMMITTER_NAME='p2p-net release snapshot' \
      GIT_COMMITTER_EMAIL='release-snapshot@invalid' \
      GIT_COMMITTER_DATE="@$SOURCE_EPOCH +0000" \
      git commit-tree "$SOURCE_TREE" -p "$BASE_COMMIT"
  )"
  [[ -n "$SNAPSHOT_COMMIT" ]] || fail "could not create deterministic release snapshot commit"
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

cleanup_worktrees
rm -rf "$REPRO_ROOT"
mkdir -p "$REPRO_ROOT"

echo "p2p-net canonical Linux release build"
echo "Root: $ROOT"

echo
echo "==> Capture exact source snapshot"
WORKING_STATUS="$(git status --porcelain=v1 --untracked-files=all)"
BASE_COMMIT="$(git rev-parse HEAD)"
BASE_TREE="$(git rev-parse HEAD^{tree})"
SOURCE_EPOCH="$(git show -s --format=%ct HEAD)"
capture_source_snapshot
CAPTURED_SOURCE_TREE="$SOURCE_TREE"
CAPTURED_RELEASE_INPUT_SHA256="$RELEASE_INPUT_SHA256"
echo "Base commit: $BASE_COMMIT"
echo "Source snapshot tree: $SOURCE_TREE"
echo "Release-input SHA-256: $RELEASE_INPUT_SHA256 ($RELEASE_INPUT_COUNT files)"
if [[ -z "$WORKING_STATUS" ]]; then
  echo "Working tree: clean"
else
  echo "Working tree: dirty; exact tracked/untracked non-ignored content will be snapshotted"
fi

create_snapshot_commit

if [[ "$FORCE_VALIDATION" == "0" ]] && find_validation_evidence "$RELEASE_INPUT_SHA256"; then
  echo
  echo "==> Reuse matching full-validation evidence"
  echo "Evidence: $VALIDATION_EVIDENCE"
  echo "Evidence kind: $VALIDATION_EVIDENCE_KIND"
else
  VALIDATION_ARGS=(--no-pause)
  if [[ "$NO_INSTALL_TOOLS" == "1" ]]; then
    VALIDATION_ARGS+=(--no-install-tools)
  fi
  echo
  echo "==> Full production validation from frozen snapshot"
  mkdir -p "$WORKTREE_ROOT"
  git worktree add --detach "$VALIDATION_WORKTREE" "$SNAPSHOT_COMMIT" >/dev/null
  bash "$VALIDATION_WORKTREE/run-full-validation.sh" "${VALIDATION_ARGS[@]}"

  TRACKED_VALIDATION_STATUS="$(git -C "$VALIDATION_WORKTREE" status --porcelain=v1 --untracked-files=no)"
  if [[ -n "$TRACKED_VALIDATION_STATUS" ]]; then
    printf 'Tracked files changed inside the validation snapshot:\n%s\n' "$TRACKED_VALIDATION_STATUS" >&2
    fail "validation modified tracked source files in the frozen snapshot"
  fi

  find_validation_evidence "$RELEASE_INPUT_SHA256" "$VALIDATION_WORKTREE" \
    || fail "full validation passed but no matching durable evidence manifest was produced for the frozen snapshot"

  SOURCE_EVIDENCE_DIR="$(dirname "$VALIDATION_EVIDENCE")"
  EVIDENCE_RUN_NAME="$(basename "$SOURCE_EVIDENCE_DIR")"
  ROOT_EVIDENCE_DIR="$ROOT/qa/evidence/runs/$EVIDENCE_RUN_NAME"
  mkdir -p "$ROOT/qa/evidence/runs"
  rm -rf "$ROOT_EVIDENCE_DIR"
  cp -a "$SOURCE_EVIDENCE_DIR" "$ROOT_EVIDENCE_DIR"
  VALIDATION_EVIDENCE="$ROOT_EVIDENCE_DIR/manifest.txt"
  VALIDATION_EVIDENCE_KIND="$(manifest_value "$VALIDATION_EVIDENCE" evidence_kind)"
  remove_worktree "$VALIDATION_WORKTREE"
fi

echo
echo "==> Resolve release identity"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
RUSTC_VERSION="$(rustc --version)"
CARGO_VERSION="$(cargo --version)"

[[ -n "$HOST_TARGET" ]] || fail "could not determine rustc host target"
[[ "$HOST_TARGET" == *-unknown-linux-* ]] || fail "Linux release runner requires a Linux Rust host target (found: $HOST_TARGET)"
[[ "$RUSTC_VERSION" == rustc\ 1.98.0\ * ]] || fail "release requires rustc 1.98.0 exactly (found: $RUSTC_VERSION)"

echo "Base commit: $BASE_COMMIT"
echo "Base tree: $BASE_TREE"
echo "Snapshot commit: $SNAPSHOT_COMMIT"
echo "Snapshot tree: $SOURCE_TREE"
echo "Target: $HOST_TARGET"
echo "SOURCE_DATE_EPOCH: $SOURCE_EPOCH"
echo "Rust: $RUSTC_VERSION"
echo "Cargo: $CARGO_VERSION"

echo
echo "==> Create independent clean source worktrees"
mkdir -p "$REPRO_ROOT" "$WORKTREE_ROOT"
git worktree add --detach "$WORKTREE_A" "$SNAPSHOT_COMMIT" >/dev/null
git worktree add --detach "$WORKTREE_B" "$SNAPSHOT_COMMIT" >/dev/null

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$WORKTREE_A/Cargo.toml" | head -n 1)"
[[ -n "$VERSION" ]] || fail "could not determine package version from frozen release snapshot"
(
  cd "$WORKTREE_A"
  cargo metadata --locked --format-version 1 >/dev/null
)

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
git ls-tree -r --full-tree "$SOURCE_TREE" >"$DIST_DIR/SOURCE-MANIFEST.txt"
SOURCE_MANIFEST_SHA256="$(sha256sum "$DIST_DIR/SOURCE-MANIFEST.txt" | awk '{print $1}')"
printf '%s  SOURCE-MANIFEST.txt\n' "$SOURCE_MANIFEST_SHA256" >"$DIST_DIR/SOURCE-MANIFEST.sha256.txt"
cp "$RELEASE_INPUTS_FILE" "$DIST_DIR/RELEASE-INPUTS.txt"
printf '%s  RELEASE-INPUTS.txt\n' "$RELEASE_INPUT_SHA256" >"$DIST_DIR/RELEASE-INPUTS.sha256.txt"
mkdir -p "$DIST_DIR/validation-evidence"
cp -a "$(dirname "$VALIDATION_EVIDENCE")/." "$DIST_DIR/validation-evidence/"
VALIDATION_EVIDENCE_SHA256="$(sha256sum "$VALIDATION_EVIDENCE" | awk '{print $1}')"
VALIDATION_EVIDENCE_RELATIVE="${VALIDATION_EVIDENCE#"$ROOT/"}"
WORKING_TREE_STATE="clean"
[[ -n "$WORKING_STATUS" ]] && WORKING_TREE_STATE="dirty-snapshotted"

cat >"$DIST_DIR/BUILD-MANIFEST.txt" <<EOF_MANIFEST
project=p2p-net
version=$VERSION
git_base_commit=$BASE_COMMIT
git_base_tree=$BASE_TREE
source_snapshot_commit=$SNAPSHOT_COMMIT
source_snapshot_tree=$SOURCE_TREE
source_manifest_sha256=$SOURCE_MANIFEST_SHA256
release_input_sha256=$RELEASE_INPUT_SHA256
release_input_file_count=$RELEASE_INPUT_COUNT
working_tree_state=$WORKING_TREE_STATE
source_date_epoch=$SOURCE_EPOCH
rustc=$RUSTC_VERSION
cargo=$CARGO_VERSION
target=$HOST_TARGET
linker=cc
profile=release
features=default,dashboard
artifact=$ARTIFACT_NAME
sha256=$HASH_A
reproducibility=verified-two-independent-clean-worktrees-from-exact-snapshot
validation=durable-evidence-pass
validation_evidence_kind=$VALIDATION_EVIDENCE_KIND
validation_evidence_path=$VALIDATION_EVIDENCE_RELATIVE
validation_evidence_sha256=$VALIDATION_EVIDENCE_SHA256
EOF_MANIFEST

echo
echo "==> Release output"
echo "Artifact: $DIST_DIR/$ARTIFACT_NAME"
echo "Manifest: $DIST_DIR/BUILD-MANIFEST.txt"
echo "Checksums: $DIST_DIR/SHA256SUMS.txt"
echo "Source manifest: $DIST_DIR/SOURCE-MANIFEST.txt"
echo "Validation evidence bundle: $DIST_DIR/validation-evidence ($VALIDATION_EVIDENCE_KIND)"
echo
echo "Canonical Linux release build completed successfully."
