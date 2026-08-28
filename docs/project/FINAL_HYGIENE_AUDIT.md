# Final hygiene audit

Date: 2026-07-02

## Scope

This audit re-ran the guideline review that produced the completed cleanup roadmap and was refreshed after the consumer public-network roadmap Step 2 added public rendezvous observability and fallback source accounting.

The audit covered:

- organized file/folder structure;
- clear separation of concerns;
- consistent naming;
- Single Responsibility Principle (SRP);
- duplicate helper implementations;
- legacy, deprecated, transitional, or stale code/docs language;
- large-file and crowded-folder hotspots;
- QA test registration after domain grouping.

## Result

Static hygiene audit result: pass.

No remaining guideline violations were found in the static audit. The original audit findings have been addressed:

1. `crates/node/mod.rs` was reduced and now delegates metrics, startup discovery preparation, runtime looping, config, validation, and snapshot responsibilities to focused modules.
2. `crates/node/types.rs` was removed and split into `config.rs`, `config_validation.rs`, and `snapshot.rs`.
3. `crates/connectivity/relay.rs` is now a facade over focused relay modules.
4. Duplicated configuration-error helpers were centralized in `crates/common/error.rs`.
5. Duplicated multiaddr classification helpers were centralized in `crates/connectivity/addr.rs`.
6. `qa/tests/` is domain-grouped instead of a flat dumping ground.

## Longest files

Longest file overall, excluding `.git/`:

```text
Cargo.lock — 5,882 lines
```

Longest runtime source files:

```text
crates/protocol/pulse.rs — 494 lines
crates/connectivity/dns.rs — 450 lines
crates/node/mod.rs — 450 lines
crates/stack/discovery.rs — 436 lines
crates/node/events/connection.rs — 435 lines
crates/bindings/mod.rs — 431 lines
crates/api/mod.rs — 426 lines
crates/connectivity/dht.rs — 404 lines
crates/node/runtime/driver.rs — 394 lines
crates/connectivity/connection_strategy.rs — 362 lines
```

Former hotspot status:

```text
crates/node/mod.rs — 450 lines
crates/node/config.rs — 261 lines
crates/node/config/listeners.rs — 53 lines
crates/node/config_validation.rs — 173 lines
crates/node/metrics.rs — 11 lines
crates/node/metrics/prometheus.rs — 24 lines
crates/node/runtime.rs — 135 lines
crates/node/runtime/driver.rs — 394 lines
crates/node/runtime/observability.rs — 38 lines
crates/node/runtime/dht_schedule.rs — 79 lines
crates/node/runtime/periodic.rs — 118 lines
crates/node/snapshot.rs — 235 lines
crates/node/snapshot/helpers.rs — 17 lines
crates/node/startup.rs — 104 lines
crates/node/startup/addrs.rs — 251 lines
crates/connectivity/relay.rs — 15 lines
crates/connectivity/relay/address.rs — 68 lines
crates/connectivity/relay/config.rs — 272 lines
crates/connectivity/relay/policy.rs — 17 lines
crates/connectivity/relay/schedule.rs — 173 lines
crates/connectivity/relay/state.rs — 128 lines
```

## Structure result

The root layout is organized and not nested. Platform applications have explicit owners instead of living inside the shared core:

```text
.git/
.github/
apps/
  windows/
  android/
Cargo.toml
Cargo.lock
README.md
build-release.cmd
build-release.sh
run-full-validation.cmd
run-full-validation.sh
crates/
docs/
examples/
external/
qa/
rust-toolchain.toml
```

`qa/tests/` is domain-grouped:

```text
qa/tests/api/
qa/tests/config/
qa/tests/discovery/
qa/tests/hygiene/
qa/tests/observability/
qa/tests/operator/
qa/tests/relay/
qa/tests/runtime/
qa/tests/security/
```

Canonical release implementation helpers live under `qa/release/`, while the user-facing Windows/Linux release launchers stay at the repository root. The desktop dashboard application is owned by `apps/windows/`; native Android JNI/service/UI code is owned by `apps/android/`; shared platform policy is split into `crates/platform/android.rs`, `desktop.rs`, and `ios.rs`. `qa/tests/hygiene/platform_app_architecture.rs` and the focused `qa/tests/hygiene/platform_app_windows.rs` integration test prevent those boundaries from collapsing.

## DRY result

Centralized helper checks:

```text
fn config_error(...) — only in crates/common/error.rs
fn has_reachable_transport(...) — only in crates/connectivity/addr.rs
fn has_unspecified_ip(...) — only in crates/connectivity/addr.rs
```

No `#[allow(...)]` cleanup suppressions were added.

## Added regression coverage

Step 9 added `qa/tests/hygiene/final_hygiene_audit.rs` and registered it in `Cargo.toml`. Canonical release reproducibility is separately guarded by `qa/tests/hygiene/release_reproducibility.rs`, which requires the root Windows/Linux release launchers to retain exact working-tree snapshotting, durable validation-evidence binding, two-worktree reproduction, deterministic linking, and checksum verification.

That test locks in the cleanup by checking:

- the original SRP hotspots stay split;
- runtime and QA Rust files stay under the hygiene line-count budget;
- duplicate cleanup helpers do not return;
- this final audit document remains present and aligned with the final module layout.

## Validation limitation

The edit sandbox does not include `cargo`, `rustc`, or `rustfmt`, so Rust compilation, formatting, clippy, cargo-deny, cargo-audit, and soak tests must be verified on a machine with the Rust toolchain installed:

```cmd
run-full-validation.cmd
```
