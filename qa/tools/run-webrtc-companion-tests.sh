#!/usr/bin/env bash
# Runs the p2p-net-webrtc companion crate's unit, smoke, and doc tests against
# the audited root Cargo.lock. The companion is deliberately outside the
# production workspace and has no committed lockfile (/external/**/Cargo.lock is
# ignored), so seed its lockfile from the root lock, let Cargo prune it to the
# companion's graph, and fail if that graph needs any package version the root
# lock does not pin.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="$ROOT/external/libp2p-webrtc/Cargo.toml"
COMPANION_LOCK="$ROOT/external/libp2p-webrtc/Cargo.lock"

lock_packages() {
  awk '/^name = /{name = $3} /^version = /{print name, $3}' "$1" | tr -d '"\r' | LC_ALL=C sort -u
}

cp "$ROOT/Cargo.lock" "$COMPANION_LOCK"
cargo metadata --format-version 1 --manifest-path "$MANIFEST" >/dev/null

unpinned="$(LC_ALL=C comm -23 <(lock_packages "$COMPANION_LOCK") <(lock_packages "$ROOT/Cargo.lock"))"
if [[ -n "$unpinned" ]]; then
  echo "ERROR: the companion test graph needs packages the audited root Cargo.lock does not pin:" >&2
  echo "$unpinned" >&2
  exit 1
fi
echo "Companion lockfile is a subset of the audited root Cargo.lock."

cargo test --manifest-path "$MANIFEST" --locked --all-features -j 1
