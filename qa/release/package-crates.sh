#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

RUST_VERSION="$(rustc +1.98.0 --version)"
[[ "$RUST_VERSION" == rustc\ 1.98.0\ * ]] || { echo "ERROR: expected Rust 1.98.0, got $RUST_VERSION" >&2; exit 1; }

printf '==> Locked workspace dependency preflight\n'
cargo +1.98.0 metadata --locked --format-version 1 >/dev/null
printf 'Committed Cargo.lock matches the current workspace graph.\n\n'

normalized_manifest_has_path_dependency() {
  awk '
    /^\[/ {
      in_dep = ($0 ~ /^\[(target\..+\.)?(dev-|build-)?dependencies(\.|\])/)
    }
    in_dep && /(^|[[:space:]{,])path[[:space:]]*=/ { found = 1 }
    END { exit(found ? 0 : 1) }
  '
}

printf '==> Package publishable crates\n'
PACKAGE_TARGET_DIR="$ROOT_DIR/target/package-crates"
rm -rf "$PACKAGE_TARGET_DIR"
mkdir -p "$PACKAGE_TARGET_DIR"
CARGO_TARGET_DIR="$PACKAGE_TARGET_DIR" cargo +1.98.0 package \
  --manifest-path "$ROOT_DIR/external/libp2p-webrtc/Cargo.toml" \
  --allow-dirty

# The root package normalizes its local path+version dependency to the crates.io
# package name. Until the companion is published, use a command-line-only patch
# for package verification; it is not serialized into the resulting .crate.
CARGO_TARGET_DIR="$PACKAGE_TARGET_DIR" cargo +1.98.0 package \
  --package p2p-net \
  --locked --allow-dirty \
  --config "patch.crates-io.p2p-net-webrtc.path='external/libp2p-webrtc'"

printf '\n==> Locate normalized crates.io payloads\n'
SUPPORT_CRATE="$PACKAGE_TARGET_DIR/package/p2p-net-webrtc-0.1.0.crate"
ROOT_CRATE="$PACKAGE_TARGET_DIR/package/p2p-net-0.1.0.crate"
[[ -f "$SUPPORT_CRATE" ]] || { echo "ERROR: missing packaged companion crate: $SUPPORT_CRATE" >&2; exit 1; }
[[ -f "$ROOT_CRATE" ]] || { echo "ERROR: missing packaged root crate: $ROOT_CRATE" >&2; exit 1; }

ROOT_ARCHIVE_ENTRIES="$(tar -tf "$ROOT_CRATE")"
for forbidden_entry in \
  'p2p-net-0.1.0/.cargo/' \
  'p2p-net-0.1.0/external/libp2p-dns/' \
  'p2p-net-0.1.0/external/libp2p-mdns-placeholder/' \
  'p2p-net-0.1.0/external/libp2p-webrtc/'
do
  if grep -Fq "$forbidden_entry" <<<"$ROOT_ARCHIVE_ENTRIES"; then
    echo "ERROR: packaged p2p-net unexpectedly contains workspace-only entry: $forbidden_entry" >&2
    exit 1
  fi
done

NORMALIZED_ROOT="$(tar -xOf "$ROOT_CRATE" p2p-net-0.1.0/Cargo.toml)"
if normalized_manifest_has_path_dependency <<<"$NORMALIZED_ROOT"; then
  echo "ERROR: packaged p2p-net still contains a path dependency" >&2
  exit 1
fi
if grep -Fq '[patch.crates-io]' <<<"$NORMALIZED_ROOT"; then
  echo "ERROR: packaged p2p-net still contains [patch.crates-io]" >&2
  exit 1
fi
if grep -Fq '[workspace]' <<<"$NORMALIZED_ROOT"; then
  echo "ERROR: packaged p2p-net unexpectedly retains the repository workspace table" >&2
  exit 1
fi
grep -Fq 'package = "p2p-net-webrtc"' <<<"$NORMALIZED_ROOT" || {
  echo "ERROR: packaged p2p-net does not depend on registry package p2p-net-webrtc" >&2
  exit 1
}
grep -Fq 'version = "0.1.0"' <<<"$NORMALIZED_ROOT" || {
  echo "ERROR: packaged p2p-net does not retain the p2p-net-webrtc 0.1.0 registry constraint" >&2
  exit 1
}

NORMALIZED_SUPPORT="$(tar -xOf "$SUPPORT_CRATE" p2p-net-webrtc-0.1.0/Cargo.toml)"
if normalized_manifest_has_path_dependency <<<"$NORMALIZED_SUPPORT" || grep -Fq '[patch.crates-io]' <<<"$NORMALIZED_SUPPORT"; then
  echo "ERROR: packaged p2p-net-webrtc is not registry-self-contained" >&2
  exit 1
fi

printf '\n==> Compile packaged payload as a downstream consumer\n'
SMOKE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/p2p-net-crates-smoke.XXXXXX")"
cleanup_smoke() { rm -rf "$SMOKE_ROOT"; }
trap cleanup_smoke EXIT
mkdir -p "$SMOKE_ROOT/unpacked" "$SMOKE_ROOT/consumer/src"
tar -xf "$SUPPORT_CRATE" -C "$SMOKE_ROOT/unpacked"
tar -xf "$ROOT_CRATE" -C "$SMOKE_ROOT/unpacked"
ROOT_PACKAGE_DIR="$SMOKE_ROOT/unpacked/p2p-net-0.1.0"
SUPPORT_PACKAGE_DIR="$SMOKE_ROOT/unpacked/p2p-net-webrtc-0.1.0"
cat > "$SMOKE_ROOT/consumer/Cargo.toml" <<EOF
[package]
name = "p2p-net-package-consumer-smoke"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
p2p-net = { path = "$ROOT_PACKAGE_DIR" }

[patch.crates-io]
p2p-net-webrtc = { path = "$SUPPORT_PACKAGE_DIR" }
EOF
cat > "$SMOKE_ROOT/consumer/src/main.rs" <<'EOF'
async fn exercise_public_api() -> Result<(), p2p_net::NetError> {
    let node = p2p_net::start_node(p2p_net::NodeConfig::default()).await?;
    let _peers = node.get_peers().await?;
    node.shutdown().await;
    Ok(())
}

fn main() {
    let _ = exercise_public_api;
}
EOF
# compile packaged p2p-net as downstream consumer
cargo +1.98.0 generate-lockfile --manifest-path "$SMOKE_ROOT/consumer/Cargo.toml"
cargo +1.98.0 check --manifest-path "$SMOKE_ROOT/consumer/Cargo.toml" --locked

DIST_DIR="$ROOT_DIR/dist/crates"
mkdir -p "$DIST_DIR"
cp "$SUPPORT_CRATE" "$DIST_DIR/p2p-net-webrtc-0.1.0.crate"
cp "$ROOT_CRATE" "$DIST_DIR/p2p-net-0.1.0.crate"
(
  cd "$DIST_DIR"
  sha256sum p2p-net-webrtc-0.1.0.crate p2p-net-0.1.0.crate > SHA256SUMS.txt
)
cat > "$DIST_DIR/PUBLISH-ORDER.txt" <<'EOF'
1. cargo +1.98.0 publish --dry-run --manifest-path external/libp2p-webrtc/Cargo.toml --registry crates-io
2. cargo +1.98.0 publish --manifest-path external/libp2p-webrtc/Cargo.toml --registry crates-io
3. Wait until crates.io/index.crates.io resolves p2p-net-webrtc 0.1.0.
4. cargo +1.98.0 publish --dry-run --package p2p-net --registry crates-io --locked
5. cargo +1.98.0 publish --package p2p-net --registry crates-io --locked
EOF

printf '\nCrates.io package payloads are normalized and downstream-consumer checked.\n'
printf 'Artifacts: %s\n' "$DIST_DIR"
printf 'Follow %s/PUBLISH-ORDER.txt for the required companion-first publish sequence.\n' "$DIST_DIR"
