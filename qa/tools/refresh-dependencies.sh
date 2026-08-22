#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if [[ "$(rustc --version)" != rustc\ 1.98.0\ * ]]; then
  echo "Dependency refresh requires rustc 1.98.0 to match production validation." >&2
  exit 1
fi

cargo update
printf '\nCargo.lock was intentionally refreshed. Review the dependency diff, then run:\n  ./run-full-validation.sh\n'
