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

- Move `snapshot_to_prometheus_metrics(...)` and metric-string helpers into `crates/node/metrics.rs`.
- Keep metrics generation based on `NodeSnapshot` only.
- Keep `NodeHandle` behavior unchanged.

Acceptance criteria:

- `crates/node/mod.rs` no longer contains Prometheus formatting logic.
- Metrics tests continue to validate the same output.
- The extracted metrics module has a single responsibility: snapshot-to-metrics rendering.

## Step 4 — Extract node startup discovery setup

Status: pending.

Goal: reduce `start_node_with_platform(...)` by moving bootstrap, DNS resolution, public fallback decisions, peer-book seeding, and relay selection into focused startup helpers.

Scope:

- Add `crates/node/startup.rs` or similarly named focused module.
- Move resolved startup address preparation out of `mod.rs`.
- Move peer-book seed recording out of `mod.rs` where practical.
- Keep the final swarm construction and task spawning visible in node orchestration.

Acceptance criteria:

- `start_node_with_platform(...)` becomes a readable orchestration flow instead of a long setup script.
- Startup helper APIs are internal to the crate.
- Public fallback, relay selection, cached peer, and rendezvous behavior remain unchanged.

## Step 5 — Extract node runtime loop responsibilities

Status: pending.

Goal: separate event-loop mechanics from startup construction.

Scope:

- Add `crates/node/runtime.rs` for the long-running select loop and heartbeat/discovery tick handling.
- Keep command routing in `crates/node/commands.rs`.
- Keep event-specific handling in `crates/node/events.rs`.
- Pass a small runtime context instead of many loose parameters.

Acceptance criteria:

- `crates/node/mod.rs` owns public node startup only.
- Runtime loop state is grouped into named context structs.
- No clippy `too_many_arguments` regressions are introduced.

## Step 6 — Split node config, snapshot, and validation types

Status: pending.

Goal: reduce `crates/node/types.rs` so each file owns one concept.

Scope:

- Move `NodeConfig` and default config values into `crates/node/config.rs`.
- Move `NodeSnapshot` and snapshot defaults into `crates/node/snapshot.rs`.
- Move config parsing/validation helpers into `crates/node/config_validation.rs` if they remain large enough to justify separation.
- Keep re-exports from `crates/node/mod.rs` so callers do not break.

Acceptance criteria:

- `crates/node/types.rs` is removed or reduced to small compatibility re-exports.
- Config validation remains deterministic and covered by existing tests.
- Snapshot fields and serialized names remain unchanged.

## Step 7 — Split relay config, state, scheduling, and address helpers

Status: pending.

Goal: reduce `crates/connectivity/relay.rs` into cohesive modules.

Scope:

- Create a `crates/connectivity/relay/` module folder if the split is large enough.
- Move relay service config and validation into `relay/config.rs`.
- Move runtime relay state into `relay/state.rs`.
- Move reservation-window and scheduling logic into `relay/schedule.rs`.
- Move relay address helpers into `relay/address.rs` if not covered by Step 2.
- Preserve existing public paths with re-exports where needed.

Acceptance criteria:

- Relay files each have one clear responsibility.
- Existing relay, mediator, public fallback, and DCUtR tests still pass.
- No duplicate relay policy checks remain.

## Step 8 — Group QA tests by domain

Status: pending.

Goal: prevent `qa/tests/` from becoming a flat dumping ground.

Scope:

- Group tests by domain while keeping Cargo test targets working.
- Candidate groups: `api`, `config`, `discovery`, `relay`, `security`, `operator`, and `observability`.
- Update `Cargo.toml` test target paths.
- Update docs that reference moved test files.

Acceptance criteria:

- Test names remain clear and domain-oriented.
- `qa/tests/` no longer contains a long flat list of unrelated files.
- Full validation discovers every moved test target.

## Step 9 — Final hygiene audit

Status: pending.

Goal: confirm the repository satisfies the guidelines after refactors.

Scope:

- Re-run the same guideline audit used to produce this roadmap.
- Check longest files and line counts again.
- Search for duplicate helpers, dead compatibility shims, old phase language, stale docs, and unused modules.
- Remove or reorganize anything that became temporary during the cleanup steps.

Acceptance criteria:

- No large SRP violations remain.
- No obvious duplicate helper implementations remain.
- Docs match the final module layout.
- Full validation passes.
