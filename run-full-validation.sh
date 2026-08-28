#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ "${P2P_VALIDATION_EVIDENCE_ACTIVE:-0}" != "1" ]]; then
  exec bash "$SCRIPT_DIR/qa/evidence/run-validation-with-evidence.sh" "$SCRIPT_DIR/run-full-validation.sh" "$@"
fi

NO_INSTALL_TOOLS=0
NO_CLEAN=0
NO_PAUSE=0
FROM_STAGE="full"
FROM_RANK=0
RESUME_NO_CLEAN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-install-tools)
      NO_INSTALL_TOOLS=1
      ;;
    --no-clean)
      NO_CLEAN=1
      ;;
    --no-pause)
      NO_PAUSE=1
      ;;
    --from)
      if [[ $# -lt 2 ]]; then
        echo "--from requires a stage name." >&2
        exit 2
      fi
      FROM_STAGE="$2"
      shift
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
  shift
done

stage_rank() {
  case "$1" in
    full) echo 0 ;;
    lockfile) echo 1 ;;
    format) echo 2 ;;
    dependency-graph) echo 3 ;;
    tests) echo 4 ;;
    dashboard) echo 5 ;;
    clippy) echo 6 ;;
    audit) echo 7 ;;
    deny) echo 8 ;;
    *) return 1 ;;
  esac
}

if ! FROM_RANK="$(stage_rank "$FROM_STAGE")"; then
  echo "Unknown --from stage: $FROM_STAGE" >&2
  echo "Valid stages: lockfile, format, dependency-graph, tests, dashboard, clippy, audit, deny" >&2
  exit 2
fi
if (( FROM_RANK > 0 )) && [[ "$NO_CLEAN" == "0" ]]; then
  NO_CLEAN=1
  RESUME_NO_CLEAN=1
fi

should_run() {
  local rank
  rank="$(stage_rank "$1")"
  (( rank >= FROM_RANK ))
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

pause_if_interactive() {
  local status="$1"
  if [[ "$NO_PAUSE" == "0" && -t 0 && -z "${CI:-}" ]]; then
    echo
    if [[ "$status" == "0" ]]; then
      read -r -p "Validation complete. Press Enter to close..." _ || true
    else
      read -r -p "Validation failed. Press Enter to close..." _ || true
    fi
  fi
}

on_exit() {
  local status=$?
  trap - EXIT
  unset CARGO_TARGET_DIR || true
  pause_if_interactive "$status"
  exit "$status"
}
trap on_exit EXIT

run_step() {
  local name="$1"
  shift
  echo
  echo "==> ${name}"
  "$@"
}

assert_pinned_rust() {
  local version
  version="$(rustc --version)"
  if [[ "$version" != rustc\ 1.98.0\ * ]]; then
    echo "p2p-net validation requires rustc 1.98.0 exactly. Active rustc: $version" >&2
    exit 1
  fi
  echo "Pinned Rust toolchain confirmed: $version"
}

cargo_tool_version() {
  cargo install --list | sed -n "s/^$1 v\([^:]*\):$/\1/p" | head -n 1
}

ensure_cargo_tool() {
  local name="$1"
  local expected="$2"
  local installed
  installed="$(cargo_tool_version "$name")"
  if [[ "$installed" == "$expected" ]]; then
    echo "$name v$expected already installed."
    return
  fi
  if [[ "$NO_INSTALL_TOOLS" == "1" ]]; then
    echo "$name v$expected is required (installed: ${installed:-missing}). Re-run without --no-install-tools or install that exact version manually." >&2
    exit 2
  fi
  run_step "Install ${name} v${expected}" cargo install "$name" --version "$expected" --locked --force
}

set_validation_target() {
  export CARGO_TARGET_DIR="$ROOT/target/full-validation/$1"
  echo "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
}

clear_validation_target() {
  unset CARGO_TARGET_DIR || true
}

run_lockfile_readonly_tool() {
  local tool_label="$1"
  shift
  local slug="${tool_label//[^A-Za-z0-9._-]/-}"
  local backup="$ROOT/target/full-validation/Cargo.lock.${slug}.backup"
  local tool_status

  [[ -f "$ROOT/Cargo.lock" ]] || {
    echo "Cargo.lock is missing before ${tool_label}." >&2
    return 1
  }

  mkdir -p "$ROOT/target/full-validation"
  cp -p "$ROOT/Cargo.lock" "$backup"

  set +e
  "$@"
  tool_status=$?
  set -e

  if ! cmp -s "$ROOT/Cargo.lock" "$backup"; then
    echo "NOTE: ${tool_label} rewrote Cargo.lock; restoring the exact pre-tool lockfile bytes."
    cp -p "$backup" "$ROOT/Cargo.lock"
  fi
  rm -f "$backup"

  if (( tool_status != 0 )); then
    return "$tool_status"
  fi

  cargo metadata --locked --format-version 1 >/dev/null
}

run_dependency_policy() {
  if cargo deny check --config qa/ci/deny.toml --help >/dev/null 2>&1; then
    cargo deny check --config qa/ci/deny.toml
  else
    cargo deny --config qa/ci/deny.toml check
  fi
}

run_format_check_readonly() {
  local backup="$ROOT/target/full-validation/Cargo.lock.format-backup"
  local format_status

  [[ -f "$ROOT/Cargo.lock" ]] || {
    echo "Cargo.lock is missing; format validation requires the committed lockfile." >&2
    return 1
  }

  mkdir -p "$ROOT/target/full-validation"
  cp -p "$ROOT/Cargo.lock" "$backup"

  set +e
  cargo fmt --all -- --check
  format_status=$?
  set -e

  if ! cmp -s "$ROOT/Cargo.lock" "$backup"; then
    echo "NOTE: cargo fmt metadata rewrote Cargo.lock; restoring the exact pre-format lockfile bytes."
    cp -p "$backup" "$ROOT/Cargo.lock"
  fi
  rm -f "$backup"

  if (( format_status != 0 )); then
    return "$format_status"
  fi

  cargo metadata --locked --format-version 1 >/dev/null
}

assert_no_rejected_dns_resolver() {
  if [[ ! -f Cargo.lock ]]; then
    echo "Cargo.lock is missing; production validation requires the committed lockfile." >&2
    exit 1
  fi

  local rejected_names=("hickory-proto" "hickory-resolver")
  local package_name
  for package_name in "${rejected_names[@]}"; do
    if grep -Fq "$package_name" Cargo.lock; then
      echo "${package_name} is present in Cargo.lock. Dependency path:" >&2
      cargo tree --target all -i "$package_name" || true
      echo "${package_name} is still present in Cargo.lock." >&2
      exit 1
    fi
    echo "${package_name} is not present in Cargo.lock."
  done
}

run_cargo_audit_with_repo_config() {
  local cargo_dir="$ROOT/.cargo"
  local staged_audit_config="$cargo_dir/audit.toml"
  local status
  mkdir -p "$cargo_dir"
  cp "$ROOT/qa/ci/audit.toml" "$staged_audit_config"
  set +e
  cargo audit --file "$ROOT/Cargo.lock"
  status=$?
  set -e
  rm -f "$staged_audit_config"
  rmdir "$cargo_dir" 2>/dev/null || true
  return "$status"
}

echo "p2p-net full stable validation"
echo "Root: $ROOT"
echo "NoInstallTools: $NO_INSTALL_TOOLS"
echo "NoClean: $NO_CLEAN"
echo "ResumeFrom: $FROM_STAGE"
if [[ "$RESUME_NO_CLEAN" == "1" ]]; then
  echo "Resume mode preserves validation artifacts; --no-clean is implied."
fi
echo
echo "This is the canonical Linux one-file validation runner. It verifies the committed lockfile and formatting without mutating source, and uses isolated target directories to avoid stale/incomplete build artifacts."

export CARGO_INCREMENTAL=0
export CARGO_BUILD_PIPELINING=false

command -v rustc >/dev/null 2>&1 || { echo "rustc was not found on PATH." >&2; exit 1; }
command -v cargo >/dev/null 2>&1 || { echo "cargo was not found on PATH." >&2; exit 1; }

run_step "Rust version" rustc --version
cargo --version
assert_pinned_rust

if [[ "$NO_CLEAN" != "1" ]]; then
  echo
  echo "==> Clean validation artifacts"
  rm -rf target/full-validation
  cargo clean
fi

[[ -f "$ROOT/Cargo.lock" ]] || {
  echo "Cargo.lock is missing; production validation requires the committed lockfile." >&2
  exit 1
}
mkdir -p "$ROOT/target/full-validation"
VALIDATION_LOCK_BASELINE="$ROOT/target/full-validation/Cargo.lock.validation-baseline"
cp -p "$ROOT/Cargo.lock" "$VALIDATION_LOCK_BASELINE"

if should_run lockfile; then
  run_step "Verify committed dependency lockfile" cargo metadata --locked --format-version 1
fi

ensure_cargo_tool cargo-audit 0.22.2
ensure_cargo_tool cargo-deny 0.20.2

if should_run format; then
  run_step "Format check" run_format_check_readonly
fi
if should_run dependency-graph; then
  run_step "Dependency graph guard" assert_no_rejected_dns_resolver
fi

if should_run tests; then
  set_validation_target tests
  echo "NOTE: The Rust harness will report three long hostile/load tests as ignored in this normal phase. They are deferred, not omitted: this runner executes each one once at the end, with the soak test last."
  run_step "Tests" cargo test --workspace --locked -j 1
fi

if should_run dashboard; then
  set_validation_target dashboard
  run_step "Dashboard feature tests" cargo test --features dashboard --locked -j 1
fi

if should_run clippy; then
  set_validation_target clippy
  run_step "Clippy" cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings
fi

clear_validation_target
if should_run audit; then
  run_step "Security audit" run_lockfile_readonly_tool cargo-audit run_cargo_audit_with_repo_config
fi

if should_run deny; then
  # cargo-deny may invoke Cargo metadata internally without exposing Cargo's
  # --locked flag. Treat it as a read-only transaction around the committed
  # lockfile and re-prove locked metadata immediately afterward.
  run_step "Dependency policy" run_lockfile_readonly_tool cargo-deny run_dependency_policy
fi

set_validation_target hostile
run_step "Deferred hostile relay-load test" cargo test --test multi_node_hostile --locked -j 1 relay_reservation_spam_does_not_panic -- --ignored --exact --nocapture
run_step "Deferred hostile connection-churn test" cargo test --test multi_node_hostile --locked -j 1 circuit_open_close_spam_does_not_hang -- --ignored --exact --nocapture
run_step "Deferred one-minute soak test (final test)" cargo test --test multi_node_hostile --locked -j 1 long_running_soak_node_stays_responsive -- --ignored --exact --nocapture

clear_validation_target
if ! cmp -s "$ROOT/Cargo.lock" "$VALIDATION_LOCK_BASELINE"; then
  echo "Cargo.lock changed during validation. The validation runner must be read-only with respect to the committed dependency lockfile." >&2
  exit 1
fi
rm -f "$VALIDATION_LOCK_BASELINE"
echo
if (( FROM_RANK > 0 )); then
  echo "p2p-net validation from $FROM_STAGE through the final soak test passed."
  echo "Earlier validation stages were intentionally skipped in resume mode."
else
  echo "All stable p2p-net validation checks passed."
fi
