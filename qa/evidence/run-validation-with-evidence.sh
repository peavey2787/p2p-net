#!/usr/bin/env bash
set -uo pipefail

LAUNCHER="$1"
shift
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-linux-$(printf '%08x' "$RANDOM$RANDOM" 2>/dev/null || echo 00000000)"
RUN_DIR="$ROOT/qa/evidence/runs/$RUN_ID"
TRANSCRIPT="$RUN_DIR/transcript.txt"
MANIFEST="$RUN_DIR/manifest.txt"
INPUTS="$RUN_DIR/release-inputs.txt"
STARTED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RESUME_FROM="full"
ARGS=("$@")

for ((i=0; i<${#ARGS[@]}; i++)); do
  if [[ "${ARGS[$i]}" == "--from" && $((i + 1)) -lt ${#ARGS[@]} ]]; then
    RESUME_FROM="${ARGS[$((i + 1))]}"
  fi
done
MODE="full"
[[ "$RESUME_FROM" == "full" ]] || MODE="resume"
mkdir -p "$RUN_DIR"

P2P_FINGERPRINT_MANIFEST_OUT="$INPUTS" bash "$ROOT/qa/evidence/source-fingerprint.sh" >"$RUN_DIR/fingerprint-before.txt"
# shellcheck disable=SC1090
source "$RUN_DIR/fingerprint-before.txt"
PRE_WORKSPACE_TREE="$workspace_tree"
PRE_RELEASE_INPUT_SHA256="$release_input_sha256"
PRE_RELEASE_INPUT_FILE_COUNT="$release_input_file_count"

printf 'p2p-net validation evidence transcript\nrun_id=%s\nstarted_utc=%s\nlauncher=%s\narguments=%q\n\n' \
  "$RUN_ID" "$STARTED_UTC" "$LAUNCHER" "${ARGS[*]}" >"$TRANSCRIPT"
echo "Validation evidence: $RUN_DIR"

export P2P_VALIDATION_EVIDENCE_ACTIVE=1
set +e
"$LAUNCHER" "${ARGS[@]}" 2>&1 | tee -a "$TRANSCRIPT"
STATUS=${PIPESTATUS[0]}
set -e
FINISHED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RESULT="fail"
[[ "$STATUS" == "0" ]] && RESULT="pass"

P2P_FINGERPRINT_MANIFEST_OUT="$RUN_DIR/release-inputs-after.txt" bash "$ROOT/qa/evidence/source-fingerprint.sh" >"$RUN_DIR/fingerprint-after.txt"
# shellcheck disable=SC1090
source "$RUN_DIR/fingerprint-after.txt"
POST_RELEASE_INPUT_SHA256="$release_input_sha256"
POST_RELEASE_INPUT_FILE_COUNT="$release_input_file_count"
RELEASE_INPUTS_STABLE=false
[[ "$POST_RELEASE_INPUT_SHA256" == "$PRE_RELEASE_INPUT_SHA256" ]] && RELEASE_INPUTS_STABLE=true

git -C "$ROOT" status --porcelain=v1 --untracked-files=all >"$RUN_DIR/git-status.txt" || true
GIT_COMMIT="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
GIT_TREE="$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo unknown)"
LOCK_HASH="$(sha256sum "$ROOT/Cargo.lock" 2>/dev/null | awk '{print $1}')"
printf '%s  Cargo.lock\n' "${LOCK_HASH:-unknown}" >"$RUN_DIR/Cargo.lock.sha256.txt"

cat >"$MANIFEST" <<EOF_MANIFEST
schema=1
evidence_kind=machine-captured
platform=linux
mode=$MODE
resume_from=$RESUME_FROM
result=$RESULT
exit_code=$STATUS
started_utc=$STARTED_UTC
finished_utc=$FINISHED_UTC
source_workspace_tree=$PRE_WORKSPACE_TREE
release_input_sha256=$PRE_RELEASE_INPUT_SHA256
release_input_file_count=$PRE_RELEASE_INPUT_FILE_COUNT
post_validation_release_input_sha256=$POST_RELEASE_INPUT_SHA256
post_validation_release_input_file_count=$POST_RELEASE_INPUT_FILE_COUNT
release_inputs_stable=$RELEASE_INPUTS_STABLE
git_commit=$GIT_COMMIT
git_tree=$GIT_TREE
git_status=$([[ -s "$RUN_DIR/git-status.txt" ]] && echo dirty || echo clean)
cargo_lock_sha256=${LOCK_HASH:-unknown}
rustc=$(rustc --version 2>/dev/null || echo unknown)
rustc_verbose=$(rustc -vV 2>/dev/null | tr '\n' ';' || echo unknown)
cargo=$(cargo --version 2>/dev/null || echo unknown)
cargo_audit=$(cargo audit --version 2>/dev/null || echo unknown)
cargo_deny=$(cargo deny --version 2>/dev/null || echo unknown)
arguments=${ARGS[*]}
transcript=transcript.txt
EOF_MANIFEST
printf '%s\n' "$RESULT" >"$RUN_DIR/$([[ "$STATUS" == "0" ]] && echo PASS || echo FAIL)"
echo "Validation evidence saved: $MANIFEST"
exit "$STATUS"
