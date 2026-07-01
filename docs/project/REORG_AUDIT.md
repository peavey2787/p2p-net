# Reorganization audit

Date: 2026-07-01

## Scope

This pass reorganized the repository to match the modular baseline:

- Runtime code under `crates/p2p-net/src/`.
- Implementation docs under `docs/impl/`.
- Core specifications under `docs/spec/`.
- Validation docs under `docs/validation/`.
- Project/audit notes under `docs/project/`.
- Deferred work under `docs/future-work/`.
- CI and master validation scripts under `qa/ci/`.
- Global/system/invariant tests under `qa/tests/`.
- Fuzzing under `qa/fuzz/`.
- QA utilities under `qa/tools/`.
- Test vectors under `qa/vectors/`.
- Local third-party patches under `external/vendor/`.

## Structural checks performed in the sandbox

- Verified TOML parsing for all `.toml` files.
- Verified JSON parsing for all `.json` files.
- Verified every `Cargo.toml` lib/example/test path exists.
- Verified old top-level directories do not remain: `src/`, `scripts/`, `unit_tests/`, `fuzz/`, `vendor/`.
- Verified `git diff --check` is clean.
- Added `codebase_hygiene` assertions for the new layout and Cargo path policy.

## Tooling limitation

The sandbox used for this edit does not include `cargo`, `rustc`, or `rustfmt`, so Rust compilation, formatting, clippy, audit, deny, and soak tests must be verified by running `qa/ci/run-full-validation.ps1` or `qa/ci/run-full-validation.sh` on a machine with the Rust toolchain installed.
