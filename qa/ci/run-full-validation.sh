#!/usr/bin/env bash
set -euo pipefail

SKIP_IGNORED=0
NO_INSTALL_TOOLS=0
NO_CLEAN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-ignored)
      SKIP_IGNORED=1
      shift
      ;;
    --no-install-tools)
      NO_INSTALL_TOOLS=1
      shift
      ;;
    --no-clean)
      NO_CLEAN=1
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

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
  shift
  if cargo_tool_installed "$name"; then
    echo "$name already installed."
    return
  fi
  if [[ "$NO_INSTALL_TOOLS" == "1" ]]; then
    echo "$name is missing. Re-run without --no-install-tools or install it manually." >&2
    exit 2
  fi
  run_step "Install ${name}" "$@"
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

  local rejected_names=("hickory-""proto" "hickory-""resolver")

  for package_name in "${rejected_names[@]}"; do
    local package_version
    package_version="$(awk -v pkg="$package_name" '
      /^\[\[package\]\]$/ { in_pkg=1; name=""; version=""; next }
      in_pkg && $0 == "name = \"" pkg "\"" { name=pkg }
      in_pkg && /^version = / { version=$0; gsub(/^version = \"|\"$/, "", version) }
      in_pkg && name == pkg && version != "" { print version; exit 0 }
    ' Cargo.lock)"

    if [[ -n "$package_version" ]]; then
      local package_spec="${package_name}@${package_version}"
      echo "${package_name} ${package_version} is present in Cargo.lock. Dependency path:" >&2
      cargo tree --target all -i "$package_spec" || true
      echo "${package_name} ${package_version} is still present in Cargo.lock." >&2
      exit 1
    fi

    echo "${package_name} is not present in Cargo.lock."
  done
}

echo "p2p-net full stable validation"
echo "Root: $ROOT"
echo "SkipIgnored: $SKIP_IGNORED"
echo "NoInstallTools: $NO_INSTALL_TOOLS"
echo "NoClean: $NO_CLEAN"
echo
echo "This script is the canonical one-command validation runner. It auto-formats with cargo fmt and uses isolated target directories to avoid stale/incomplete artifact errors."

export CARGO_INCREMENTAL=0
export CARGO_BUILD_PIPELINING=false

run_step "Rust version" rustc --version
cargo --version
assert_stable_rust

if [[ "$NO_CLEAN" != "1" ]]; then
  run_step "Clean validation artifacts" bash -lc 'rm -rf target/full-validation && cargo clean'
fi

run_step "Refresh dependency lockfile" bash -lc 'rm -f Cargo.lock && cargo generate-lockfile'

ensure_cargo_tool cargo-audit cargo install cargo-audit --locked
ensure_cargo_tool cargo-deny cargo install cargo-deny --locked

run_step "Format" cargo fmt
run_step "Dependency graph guard" assert_no_rejected_dns_resolver

run_step "Tests" bash -lc 'export CARGO_TARGET_DIR="$PWD/target/full-validation/tests"; cargo test --workspace --locked -j 1'
run_step "Dashboard feature tests" bash -lc 'export CARGO_TARGET_DIR="$PWD/target/full-validation/dashboard"; cargo test --features dashboard --locked -j 1'
run_step "Clippy" bash -lc 'export CARGO_TARGET_DIR="$PWD/target/full-validation/clippy"; cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings'
run_step "Security audit" cargo audit --config qa/ci/audit.toml
run_step "Dependency policy" cargo deny --config qa/ci/deny.toml check

if [[ "$SKIP_IGNORED" != "1" ]]; then
  run_step "Ignored load/soak tests" bash -lc 'export CARGO_TARGET_DIR="$PWD/target/full-validation/ignored"; cargo test --test multi_node_hostile --locked -j 1 -- --ignored --nocapture'
fi

echo
echo "All stable p2p-net validation checks passed."
