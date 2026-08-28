#!/usr/bin/env bash
set -euo pipefail

rustc_version="$(rustup run 1.98.0 rustc --version)"
[[ "$rustc_version" == rustc\ 1.98.0\ * ]] || {
  printf 'ERROR: Android builds require rustc 1.98.0, got: %s\n' "$rustc_version" >&2
  exit 1
}

cargo_ndk_version="$(rustup run 1.98.0 cargo ndk --version 2>&1)"
[[ "$cargo_ndk_version" == *"4.1.2"* ]] || {
  printf 'ERROR: Android builds require cargo-ndk 4.1.2, got: %s\n' "$cargo_ndk_version" >&2
  exit 1
}

printf 'Pinned Android Rust tooling verified: %s; cargo-ndk 4.1.2\n' "$rustc_version"
