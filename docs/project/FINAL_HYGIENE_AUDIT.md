# Final hygiene audit

Date: 2026-07-01

## Scope

This audit re-ran the guideline review that produced `docs/roadmap.md` and checked the repository after Steps 1 through 8 of the cleanup roadmap.

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
Cargo.lock — 4,110 lines
```

Longest runtime source files:

```text
crates/connectivity/dns.rs — 471 lines
crates/node/mod.rs — 421 lines
crates/stack/discovery.rs — 417 lines
crates/bindings/mod.rs — 415 lines
crates/protocol/pulse.rs — 330 lines
crates/node/events/connection.rs — 321 lines
crates/connectivity/rendezvous.rs — 321 lines
crates/api/mod.rs — 321 lines
crates/connectivity/relay_discovery.rs — 309 lines
crates/node/environment.rs — 307 lines
```

Former hotspot status:

```text
crates/node/mod.rs — 421 lines
crates/node/config.rs — 236 lines
crates/node/config_validation.rs — 161 lines
crates/node/metrics.rs — 11 lines
crates/node/metrics/prometheus.rs — 222 lines
crates/node/snapshot.rs — 204 lines
crates/connectivity/relay.rs — 13 lines
crates/connectivity/relay/address.rs — 67 lines
crates/connectivity/relay/config.rs — 272 lines
crates/connectivity/relay/schedule.rs — 173 lines
crates/connectivity/relay/state.rs — 98 lines
```

## Structure result

The root layout is organized and not nested:

```text
.git/
.github/
Cargo.toml
Cargo.lock
README.md
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

## DRY result

Centralized helper checks:

```text
fn config_error(...) — only in crates/common/error.rs
fn has_reachable_transport(...) — only in crates/connectivity/addr.rs
fn has_unspecified_ip(...) — only in crates/connectivity/addr.rs
```

No `#[allow(...)]` cleanup suppressions were added.

## Added regression coverage

Step 9 added `qa/tests/hygiene/final_hygiene_audit.rs` and registered it in `Cargo.toml`.

That test locks in the cleanup by checking:

- the original SRP hotspots stay split;
- runtime and QA Rust files stay under the hygiene line-count budget;
- duplicate cleanup helpers do not return;
- this final audit document remains present and aligned with the final module layout.

## Validation limitation

The edit sandbox does not include `cargo`, `rustc`, or `rustfmt`, so Rust compilation, formatting, clippy, cargo-deny, cargo-audit, and soak tests must be verified on a machine with the Rust toolchain installed:

```powershell
.\qa\ci\run-full-validation.ps1
```
