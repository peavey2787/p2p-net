#!/usr/bin/env bash
# Runs a companion crate's (external/<dir>, e.g. libp2p-relay) unit, smoke, and
# doc tests against the audited root Cargo.lock. Usage:
#   run-companion-tests.sh <dir under external/>
# Companions are deliberately outside the
# production workspace and have no committed lockfile (/external/**/Cargo.lock is
# ignored), so seed its lockfile from the root lock, let Cargo prune it to the
# companion's graph, and fail if that graph needs any package version the root
# lock does not pin.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPANION="${1:?usage: run-companion-tests.sh <dir under external/>}"
MANIFEST="$ROOT/external/$COMPANION/Cargo.toml"
COMPANION_LOCK="$ROOT/external/$COMPANION/Cargo.lock"
[[ -f "$MANIFEST" ]] || { echo "ERROR: no companion manifest at $MANIFEST" >&2; exit 1; }

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
