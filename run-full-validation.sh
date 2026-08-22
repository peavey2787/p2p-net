#!/usr/bin/env bash
set -euo pipefail

SKIP_IGNORED=0
NO_INSTALL_TOOLS=0
NO_CLEAN=0
NO_PAUSE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-ignored)
      SKIP_IGNORED=1
      ;;
    --no-install-tools)
      NO_INSTALL_TOOLS=1
      ;;
    --no-clean)
      NO_CLEAN=1
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

assert_stable_rust() {
  local version
  version="$(rustc --version)"
  if [[ "$version" =~ nightly|beta|dev ]]; then
    echo "p2p-net validation must run on stable Rust only. Active rustc: $version" >&2
    exit 1
  fi
  echo "Stable Rust toolchain confirmed: $version"
}

cargo_tool_installed() {
  cargo install --list | grep -q "^$1 v"
}

ensure_cargo_tool() {
  local name="$1"
  if cargo_tool_installed "$name"; then
    echo "$name already installed."
    return
  fi
  if [[ "$NO_INSTALL_TOOLS" == "1" ]]; then
    echo "$name is missing. Re-run without --no-install-tools or install it manually." >&2
    exit 2
  fi
  run_step "Install ${name}" cargo install "$name" --locked
}

set_validation_target() {
  export CARGO_TARGET_DIR="$ROOT/target/full-validation/$1"
  echo "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
}

clear_validation_target() {
  unset CARGO_TARGET_DIR || true
}

assert_no_rejected_dns_resolver() {
  if [[ ! -f Cargo.lock ]]; then
    echo "Cargo.lock is missing after lockfile refresh." >&2
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
  cargo audit
  status=$?
  set -e
  rm -f "$staged_audit_config"
  rmdir "$cargo_dir" 2>/dev/null || true
  return "$status"
}

echo "p2p-net full stable validation"
echo "Root: $ROOT"
echo "SkipIgnored: $SKIP_IGNORED"
echo "NoInstallTools: $NO_INSTALL_TOOLS"
echo "NoClean: $NO_CLEAN"
echo
echo "This is the canonical Linux one-file validation runner. It auto-formats with cargo fmt and uses isolated target directories to avoid stale/incomplete build artifacts."

export CARGO_INCREMENTAL=0
export CARGO_BUILD_PIPELINING=false

command -v rustc >/dev/null 2>&1 || { echo "rustc was not found on PATH." >&2; exit 1; }
command -v cargo >/dev/null 2>&1 || { echo "cargo was not found on PATH." >&2; exit 1; }

run_step "Rust version" rustc --version
cargo --version
assert_stable_rust

if [[ "$NO_CLEAN" != "1" ]]; then
  echo
  echo "==> Clean validation artifacts"
  rm -rf target/full-validation
  cargo clean
fi

run_step "Refresh dependency lockfile" bash -c 'rm -f Cargo.lock && cargo generate-lockfile'

ensure_cargo_tool cargo-audit
ensure_cargo_tool cargo-deny

run_step "Format" cargo fmt
run_step "Dependency graph guard" assert_no_rejected_dns_resolver

set_validation_target tests
run_step "Tests" cargo test --workspace --locked -j 1

set_validation_target dashboard
run_step "Dashboard feature tests" cargo test --features dashboard --locked -j 1

set_validation_target clippy
run_step "Clippy" cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings

clear_validation_target
run_step "Security audit" run_cargo_audit_with_repo_config
run_step "Dependency policy" cargo deny check --config qa/ci/deny.toml

if [[ "$SKIP_IGNORED" != "1" ]]; then
  set_validation_target ignored
  run_step "Ignored load/soak tests" cargo test --test multi_node_hostile --locked -j 1 -- --ignored --nocapture
fi

clear_validation_target
echo
echo "All stable p2p-net validation checks passed."
