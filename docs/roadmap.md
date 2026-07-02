# Roadmap: codebase hygiene and SRP cleanup

This roadmap replaces the completed discovery-fallback roadmap. It tracks the cleanup work required by the Phase 8 audit so the repository matches the project guidelines:

- organized file/folder structure;
- clear separation of concerns;
- consistent naming;
- Single Responsibility Principle (SRP);
- no legacy, deprecated, duplicated, or unused code;
- large files split before they become dumping grounds;
- crowded folders grouped by domain.

## Current audit findings

Status before this roadmap started:

1. `crates/node/mod.rs` is too large and mixes node startup, runtime orchestration, peer-book seeding, discovery setup, relay selection, snapshot updates, heartbeat publishing, and Prometheus formatting.
2. `crates/node/types.rs` is too large and mixes user config, validation helpers, node snapshots, snapshot defaults, and multiaddr parsing helpers.
3. `crates/connectivity/relay.rs` is too large and mixes relay service config, relay state, policy validation, reservation scheduling, address helpers, and denial classification.
4. Configuration error construction was duplicated across many modules.
5. Multiaddr reachability helper logic was duplicated between peer cache and relay discovery code.
6. `qa/tests/` has grown into a flat folder with too many unrelated integration tests.

## Cleanup rules

Every step must preserve the public API and existing behavior unless the step explicitly states otherwise.

Each implementation step must:

- keep commits focused on one violation class;
- avoid adding `#[allow(...)]` suppressions for cleanup warnings;
- keep docs accurate with the code moved in that step;
- run the canonical validation command before marking the step complete:

```powershell
.\qa\ci\run-full-validation.ps1
```

## Step 1 — Centralize configuration error helpers

Status: implemented; pending full validation.

Goal: remove duplicated local `config_error(...)` constructors and keep configuration error construction in one place.

Scope:

- Add shared configuration-error constructors to `crates/common/error.rs`.
- Replace repeated local helpers in connectivity, protocol, and node modules.
- Preserve specialized error paths such as `<mediator>` and `<capability-resolver>` through a shared path-aware constructor.
- Do not change runtime behavior or public API.

Acceptance criteria:

- `grep -R "fn config_error" crates` returns only the shared helper in `crates/common/error.rs`.
- Existing validation error messages keep their previous path labels.
- Full validation passes.

## Step 2 — Centralize multiaddr classification helpers

Status: implemented; pending full validation.

Goal: remove duplicated multiaddr reachability checks and make address classification reusable.

Scope:

- Add a focused helper module, likely `crates/connectivity/addr.rs` or `crates/common/multiaddr.rs`.
- Move shared helpers such as `has_reachable_transport(...)` and `has_unspecified_ip(...)` out of peer-cache and relay-discovery modules.
- Keep peer-cache persistence logic in `peer_cache.rs`.
- Keep relay-selection logic in `relay_discovery.rs`.

Acceptance criteria:

- No duplicated implementations of reachable-transport or unspecified-IP checks remain.
- Peer-cache and relay-discovery tests still pass.
- No new public API surface is added unless required internally by crate visibility.

## Step 3 — Extract node metrics from `crates/node/mod.rs`

Status: implemented; pending full validation.

Goal: remove Prometheus formatting from node startup/orchestration.

Scope:

- Move `snapshot_to_prometheus_metrics(...)` behind the focused `crates/node/metrics.rs` facade.
- Keep Prometheus text rendering in `crates/node/metrics/prometheus.rs` so the facade remains small.
- Keep metrics generation based on `NodeSnapshot` only.
- Keep `NodeHandle` behavior unchanged.

Acceptance criteria:

- `crates/node/mod.rs` no longer contains Prometheus formatting logic.
- Metrics tests continue to validate the same output.
- The extracted metrics module has a single responsibility: snapshot-to-metrics rendering.
- `crates/node/metrics.rs` remains a small facade below the final hygiene line budget.

## Step 4 — Extract node startup discovery setup

Status: implemented; pending full validation.

Goal: reduce `start_node_with_platform(...)` by moving bootstrap, DNS resolution, public fallback decisions, peer-book seeding, and relay selection into focused startup helpers.

Scope:

- Add `crates/node/startup.rs` or similarly named focused module.
- Move resolved startup address preparation out of `mod.rs`.
- Move peer-book seed recording out of `mod.rs` where practical.
- Keep the final swarm construction and task spawning visible in node orchestration.

Implemented notes:

- Added `crates/node/startup.rs` for startup DNS resolution, cached peer loading, public fallback decisions, startup relay selection, and initial peer-book seeding.
- `crates/node/mod.rs` now asks the startup module for a `StartupDiscoverySetup`, then keeps visible orchestration steps for swarm seeding, relay reservation, rendezvous refresh, DHT startup, snapshot initialization, and task spawning.

Acceptance criteria:

- `start_node_with_platform(...)` becomes a readable orchestration flow instead of a long setup script.
- Startup helper APIs are internal to the crate.
- Public fallback, relay selection, cached peer, and rendezvous behavior remain unchanged.

## Step 5 — Extract node runtime loop responsibilities

Status: implemented; pending full validation.

Goal: separate event-loop mechanics from startup construction.

Scope:

- Add `crates/node/runtime.rs` for the long-running select loop and heartbeat/discovery tick handling.
- Keep command routing in `crates/node/commands.rs`.
- Keep event-specific handling in `crates/node/events.rs`.
- Pass a small runtime context instead of many loose parameters.

Implemented notes:

- Added `crates/node/runtime.rs` for the long-running tokio select loop, heartbeat ticks, command dispatch, swarm-event dispatch, and runtime-only state initialization.
- `crates/node/mod.rs` now constructs startup state and hands it to `runtime::spawn_node_runtime(...)` through `NodeRuntimeContext`.
- Runtime-only mutable state is grouped in `RuntimeState` instead of staying as loose local variables in node startup.

Acceptance criteria:

- `crates/node/mod.rs` owns public node startup only.
- Runtime loop state is grouped into named context structs.
- No clippy `too_many_arguments` regressions are introduced.

## Step 6 — Split node config, snapshot, and validation types

Status: implemented; pending full validation.

Goal: reduce `crates/node/types.rs` so each file owns one concept.

Scope:

- Move `NodeConfig` and default config values into `crates/node/config.rs`.
- Move `NodeSnapshot` and snapshot defaults into `crates/node/snapshot.rs`.
- Move config parsing/validation helpers into `crates/node/config_validation.rs` if they remain large enough to justify separation.
- Keep re-exports from `crates/node/mod.rs` so callers do not break.

Implemented notes:

- Added `crates/node/config.rs` for `NodeConfig`, defaults, config file loading, JSON rendering, profile/environment resolution helpers, and typed multiaddr accessors.
- Added `crates/node/config_validation.rs` for deterministic config validation and shared multiaddr parsing helpers.
- Added `crates/node/snapshot.rs` for `NodeSnapshot`, relay-state snapshot application, and network labels.
- Removed the mixed `crates/node/types.rs` file and re-exported `NodeConfig` / `NodeSnapshot` from focused modules through `crates/node/mod.rs`.

Acceptance criteria:

- `crates/node/types.rs` is removed or reduced to small compatibility re-exports.
- Config validation remains deterministic and covered by existing tests.
- Snapshot fields and serialized names remain unchanged.

## Step 7 — Split relay config, state, scheduling, and address helpers

Status: implemented; pending full validation.

Goal: reduce `crates/connectivity/relay.rs` into cohesive modules.

Scope:

- Create a `crates/connectivity/relay/` module folder if the split is large enough.
- Move relay service config and validation into `relay/config.rs`.
- Move runtime relay state into `relay/state.rs`.
- Move reservation-window and scheduling logic into `relay/schedule.rs`.
- Move relay address helpers into `relay/address.rs` if not covered by Step 2.
- Preserve existing public paths with re-exports where needed.

Implemented notes:

- Kept `crates/connectivity/relay.rs` as a 13-line facade that declares focused relay submodules and re-exports the same public relay API paths.
- Added `crates/connectivity/relay/config.rs` for `RelayServiceConfig`, `RelayAccess`, validation, peer allow/deny helpers, and libp2p relay config conversion.
- Added `crates/connectivity/relay/schedule.rs` for `RelaySchedule`, `RelayWindow`, UTC window matching, and schedule validation.
- Added `crates/connectivity/relay/state.rs` for `RelayState`, `RelayReservationPlan`, `RelayServiceHealth`, NAT updates, and denial-health classification.
- Added `crates/connectivity/relay/address.rs` for relay reservation address construction and relayed multiaddr inspection.

Acceptance criteria:

- Relay files each have one clear responsibility.
- Existing relay, mediator, public fallback, and DCUtR tests still pass.
- No duplicate relay policy checks remain.

## Step 8 — Group QA tests by domain

Status: implemented; pending full validation.

Goal: prevent `qa/tests/` from becoming a flat dumping ground.

Scope:

- Group tests by domain while keeping Cargo test targets working.
- Candidate groups: `api`, `config`, `discovery`, `relay`, `security`, `operator`, and `observability`.
- Update `Cargo.toml` test target paths.
- Update docs that reference moved test files.

Implemented notes:

- Moved all integration tests out of the `qa/tests/` root into focused domain folders: `api`, `config`, `discovery`, `hygiene`, `observability`, `operator`, `relay`, `runtime`, and `security`.
- Updated all root `Cargo.toml` `[[test]]` paths to preserve the same test target names after the move.
- Updated validation, implementation, README, and project audit docs that named moved test files.
- Updated `codebase_hygiene` so it recursively validates registered tests and rejects root-level `.rs` files under `qa/tests/`.

Acceptance criteria:

- Test names remain clear and domain-oriented.
- `qa/tests/` no longer contains a long flat list of unrelated files.
- Full validation discovers every moved test target.

## Step 9 — Final hygiene audit

Status: implemented; pending full validation.

Goal: confirm the repository satisfies the guidelines after refactors.

Scope:

- Re-run the same guideline audit used to produce this roadmap.
- Check longest files and line counts again.
- Search for duplicate helpers, dead compatibility shims, old phase language, stale docs, and unused modules.
- Remove or reorganize anything that became temporary during the cleanup steps.

Implemented notes:

- Added `docs/project/FINAL_HYGIENE_AUDIT.md` with the final guideline audit result, current longest-file report, former hotspot status, and validation limitation.
- Added `qa/tests/hygiene/final_hygiene_audit.rs` to lock in the final cleanup invariants.
- Registered the final hygiene audit test in `Cargo.toml`.
- Confirmed the original duplicate helper implementations remain centralized.
- Confirmed `qa/tests/` remains domain-grouped and root-level integration test files did not return.

Final line-count snapshot:

```text
Cargo.lock — 4,110 lines
crates/connectivity/dns.rs — 471 lines
qa/tests/hygiene/codebase_hygiene.rs — 464 lines
crates/node/mod.rs — 421 lines
crates/stack/discovery.rs — 417 lines
crates/bindings/mod.rs — 415 lines
crates/connectivity/relay.rs — 13 lines
```

Acceptance criteria:

- No large SRP violations remain.
- No obvious duplicate helper implementations remain.
- Docs match the final module layout.
- Full validation passes.
