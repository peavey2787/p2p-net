#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TREE="${1:-}"
TMP_INDEX=""
TMP_GIT_DIR=""
OLD_INDEX="${GIT_INDEX_FILE-}"
FINGERPRINT_MODE="git-worktree"
RELEASE_INPUT_PATHS=(
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  .cargo/config.toml
  crates
  apps
  external
  examples
  assets
)

cleanup() {
  if [[ -n "$TMP_INDEX" ]]; then
    rm -f "$TMP_INDEX"
  fi
  if [[ -n "$TMP_GIT_DIR" ]]; then
    rm -rf "$TMP_GIT_DIR"
  fi
  if [[ -n "$OLD_INDEX" ]]; then
    export GIT_INDEX_FILE="$OLD_INDEX"
  else
    unset GIT_INDEX_FILE || true
  fi
}
trap cleanup EXIT

cd "$ROOT"
GIT_TREE_ARGS=()
if [[ -z "$TREE" ]]; then
  if git rev-parse --verify 'HEAD^{tree}' >/dev/null 2>&1; then
    TMP_INDEX="$(mktemp "${TMPDIR:-/tmp}/p2p-net-index.XXXXXX")"
    rm -f "$TMP_INDEX"
    export GIT_INDEX_FILE="$TMP_INDEX"
    git read-tree HEAD
    git add -A
    TREE="$(git write-tree)"
  else
    # GitHub/source ZIPs do not contain .git. Create a temporary object database
    # and fingerprint the release inputs without mutating the extracted source.
    FINGERPRINT_MODE="synthetic-worktree"
    unset GIT_INDEX_FILE || true
    TMP_GIT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/p2p-net-fingerprint.XXXXXX.git")"
    git init --bare -q "$TMP_GIT_DIR"
    GIT_TREE_ARGS=(
      -c core.autocrlf=false
      -c core.filemode=false
      --git-dir="$TMP_GIT_DIR"
      --work-tree="$ROOT"
    )
    # A manifest-only handoff may intentionally omit Cargo.lock. A never-existing
    # Git pathspec is fatal, so stage only release inputs present in the archive.
    EXISTING_RELEASE_INPUT_PATHS=()
    for path in "${RELEASE_INPUT_PATHS[@]}"; do
      if [[ -e "$ROOT/$path" ]]; then
        EXISTING_RELEASE_INPUT_PATHS+=("$path")
      fi
    done
    if [[ ${#EXISTING_RELEASE_INPUT_PATHS[@]} -eq 0 ]]; then
      echo "no release inputs were found for source-archive fingerprinting" >&2
      exit 1
    fi
    git "${GIT_TREE_ARGS[@]}" add -A -- "${EXISTING_RELEASE_INPUT_PATHS[@]}"
    TREE="$(git "${GIT_TREE_ARGS[@]}" write-tree)"
  fi
fi

MANIFEST_FILE="$(mktemp "${TMPDIR:-/tmp}/p2p-net-release-inputs.XXXXXX")"
trap 'rm -f "$MANIFEST_FILE"; cleanup' EXIT
if [[ ${#GIT_TREE_ARGS[@]} -gt 0 ]]; then
  git "${GIT_TREE_ARGS[@]}" ls-tree -r --full-tree "$TREE" -- "${RELEASE_INPUT_PATHS[@]}" >"$MANIFEST_FILE"
else
  git ls-tree -r --full-tree "$TREE" -- "${RELEASE_INPUT_PATHS[@]}" >"$MANIFEST_FILE"
fi
DIGEST="$(sha256sum "$MANIFEST_FILE" | awk '{print $1}')"
COUNT="$(wc -l <"$MANIFEST_FILE" | tr -d ' ')"

printf 'workspace_tree=%s\n' "$TREE"
printf 'fingerprint_mode=%s\n' "$FINGERPRINT_MODE"
printf 'release_input_sha256=%s\n' "$DIGEST"
printf 'release_input_file_count=%s\n' "$COUNT"
printf 'release_input_manifest=%s\n' "$MANIFEST_FILE"
# The caller may copy the manifest before this process exits.
if [[ -n "${P2P_FINGERPRINT_MANIFEST_OUT:-}" ]]; then
  cp "$MANIFEST_FILE" "$P2P_FINGERPRINT_MANIFEST_OUT"
fi
