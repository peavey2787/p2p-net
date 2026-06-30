#!/usr/bin/env bash
set -euo pipefail

echo "Installing cargo-audit with locked dependencies..."
cargo install cargo-audit --locked

echo "Installing cargo-deny with locked dependencies..."
cargo install cargo-deny --locked

echo
echo "Installed stable-compatible validation tools."
echo "cargo-fuzz is intentionally not installed by this stable-tooling script."
