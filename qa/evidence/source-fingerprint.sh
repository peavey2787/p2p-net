#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TREE="${1:-}"
TMP_INDEX=""
OLD_INDEX="${GIT_INDEX_FILE-}"

cleanup() {
  if [[ -n "$TMP_INDEX" ]]; then
    rm -f "$TMP_INDEX"
  fi
  if [[ -n "$OLD_INDEX" ]]; then
    export GIT_INDEX_FILE="$OLD_INDEX"
  else
    unset GIT_INDEX_FILE || true
  fi
}
trap cleanup EXIT

cd "$ROOT"
if [[ -z "$TREE" ]]; then
  TMP_INDEX="$(mktemp "${TMPDIR:-/tmp}/p2p-net-index.XXXXXX")"
  rm -f "$TMP_INDEX"
  export GIT_INDEX_FILE="$TMP_INDEX"
  git read-tree HEAD
  git add -A
  TREE="$(git write-tree)"
fi

MANIFEST_FILE="$(mktemp "${TMPDIR:-/tmp}/p2p-net-release-inputs.XXXXXX")"
trap 'rm -f "$MANIFEST_FILE"; cleanup' EXIT
git ls-tree -r --full-tree "$TREE" -- \
  Cargo.toml Cargo.lock rust-toolchain.toml .cargo/config.toml crates apps external examples assets \
  >"$MANIFEST_FILE"
DIGEST="$(sha256sum "$MANIFEST_FILE" | awk '{print $1}')"
COUNT="$(wc -l <"$MANIFEST_FILE" | tr -d ' ')"

printf 'workspace_tree=%s\n' "$TREE"
printf 'release_input_sha256=%s\n' "$DIGEST"
printf 'release_input_file_count=%s\n' "$COUNT"
printf 'release_input_manifest=%s\n' "$MANIFEST_FILE"
# The caller may copy the manifest before this process exits.
if [[ -n "${P2P_FINGERPRINT_MANIFEST_OUT:-}" ]]; then
  cp "$MANIFEST_FILE" "$P2P_FINGERPRINT_MANIFEST_OUT"
fi
