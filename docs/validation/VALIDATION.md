# Stable validation and hostile-network testing

Run the full stable validation script from the crate root.

Windows CMD:

```cmd
run-full-validation.cmd
```

Bash:

```bash
./run-full-validation.sh
```

The script runs stable validation with DNS enabled by default through p2p-net's own startup resolver. Configured and cached `/dns`, `/dns4`, `/dns6`, and `/dnsaddr` peer addresses are resolved before dialing. Because WebSocket support in rust-libp2p 0.56 expects `libp2p-dns`, p2p-net patches that adapter to a local no-Hickory implementation and patches disallowed mDNS to a local no-op placeholder. Hickory DNS packages are rejected from `Cargo.lock`. `/dnsaddr` uses the configurable bounded DoH policy documented in `docs/impl/DNSADDR_DOH.md`.

The script remains the canonical one-command runner after the profile/environment refactor. Unit tests such as `environment_detection`, `capability_resolver`, `mediator_role`, `event_responsibility`, `behaviour_policy`, `relay_discovery`, `dcutr_policy`, `platform_runtime`, `bindings`, `codebase_hygiene`, and `codebase_architecture_hygiene` are picked up by `cargo test --workspace`, so you do not need separate commands.

The script regenerates the dependency lockfile, auto-formats the tree, runs the dependency graph guard, then runs the stable checks with isolated target directories:

```text
cargo generate-lockfile
cargo fmt
cargo test --workspace --locked -j 1
cargo test --features dashboard --locked -j 1
cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings
cargo audit  # root launcher stages qa/ci/audit.toml to .cargo/audit.toml
cargo deny --config qa/ci/deny.toml check
cargo test --test multi_node_hostile --locked -j 1 -- --ignored --nocapture
```

Defaults:

- Missing `cargo-audit` and `cargo-deny` are installed automatically unless `--no-install-tools` is used.
- `Cargo.lock` is regenerated first so stale lockfile entries, including old DNS resolver packages, are removed before audit.
- `rust-toolchain.toml` pins the stable toolchain, and the script rejects nightly/beta/dev rustc builds before running validation.
- Fuzz targets are included under `qa/fuzz/`, but they are not run by the stable one-file validation script.

Windows CMD options:

```cmd
run-full-validation.cmd --skip-ignored
run-full-validation.cmd --no-install-tools
run-full-validation.cmd --no-clean
```

Bash options:

```bash
./run-full-validation.sh --skip-ignored
./run-full-validation.sh --no-install-tools
./run-full-validation.sh --no-clean
```


Linux packet-loss/latency simulation:

```bash
sudo ./qa/tools/netem-linux.sh lo start
cargo test --test multi_node_hostile -- --ignored --nocapture
sudo ./qa/tools/netem-linux.sh lo stop
```

`netem-linux.sh` requires Linux `tc` and root privileges. Windows should skip netem tests.

## Event split validation

`event_responsibility` verifies that swarm event handling remains split by responsibility and that the dispatcher does not regain moved relay/DCUtR/gossip handlers.

## Behaviour policy validation

`behaviour_policy` verifies that the stack builder consumes `ResolvedNodeConfig`, that Kademlia server mode is no longer hard-coded for every node, and that lite/mobile-lite profiles resolve to Kademlia client mode while retaining relay-client/DCUtR fallback.


## Relay discovery validation

`qa/tests/relay/relay_discovery.rs` covers relay candidate filtering, source ordering, duplicate removal, policy validation, and lite-profile resolution. The test is registered in `Cargo.toml`, so `run-full-validation.cmd` picks it up through the existing `cargo test --workspace --locked -j 1` step.


## DCUtR policy validation

`qa/tests/relay/dcutr_policy.rs` covers default DCUtR policy safety, disabling the DCUtR capability, rejecting upgrade-without-fallback config, resolved retry policy fields, behaviour-level allowlist/cooldown wiring, and snapshot relay-fallback counters. The test is registered in `Cargo.toml`, so `run-full-validation.cmd` picks it up through the existing `cargo test --workspace --locked -j 1` step.

`platform_runtime` is picked up by `cargo test --workspace`.

## Application API validation

`qa/tests/discovery/discovery_namespace.rs` verifies hashed app discovery namespace derivation, duplicate tag removal, readable-debug guardrails, and node-config validation. The test is registered in `Cargo.toml`, so the canonical validation script picks it up automatically.

`qa/tests/api/api_primitives.rs` verifies the six public data-plane primitives plus `get_metrics()` on `NodeHandle`, app-topic namespacing and validation, `AppMessage` encode/decode behavior, addressed-message filtering, telemetry scoping, and payload size bounds. The test is registered in `Cargo.toml`, so the canonical validation script picks it up automatically.

## Codebase hygiene validation

`qa/tests/hygiene/codebase_hygiene.rs` guards repository/layout and profile-decision cleanup. `qa/tests/hygiene/codebase_architecture_hygiene.rs` guards focused node/relay module ownership and verifies every integration test under `qa/tests/` is registered exactly once in `Cargo.toml`.

## Binding facade validation

`qa/tests/api/bindings.rs` covers the cross-platform binding-safe facade:

- the support matrix confirms one shared Rust core and no separate node implementations
- Android runtime specs resolve `profile = auto` to `mobile_lite`
- iOS runtime specs flag memory storage as testing-only
- desktop runtime specs preserve listener capability and desktop filesystem storage
- config and snapshot JSON helpers work for host UI layers

The test is registered in `Cargo.toml`, so `run-full-validation.cmd` and `run-full-validation.sh` pick it up through the existing `cargo test --workspace --locked -j 1` step. Host-language generated binding tests for Kotlin/Swift should live in the app shell once a generator such as UniFFI or a C ABI wrapper is chosen.


## Operator docs and examples checks

`qa/tests/operator/operator_docs.rs` verifies that operator deployment guides exist, consumer-default, private-infrastructure-first, and public-fallback example configs parse as `NodeConfig`, and the examples preserve the expected public-fallback defaults and private-mode override.

## Manual audit and deny commands

`cargo-audit` reads repository audit configuration from `.cargo/audit.toml` in the installed version this project validates against. The canonical validation scripts keep `qa/ci/audit.toml` as the source file and stage it to `.cargo/audit.toml` only while `cargo audit` runs.

The canonical launchers invoke `cargo-deny` directly with the repository configuration and adapt to the installed CLI option placement:

```text
cargo deny --config qa/ci/deny.toml check
```

The root validation launchers detect older `cargo-deny` releases that instead require `cargo deny check --config qa/ci/deny.toml` and select the accepted form automatically.

This avoids Windows batch subroutine/label handling entirely while remaining compatible with both cargo-deny CLI layouts seen across local and hosted CI toolchains. Use `run-full-validation.cmd` or `run-full-validation.sh` for the exact flow.

## Public fallback checks

`qa/tests/discovery/discovery.rs` and `qa/tests/relay/relay_discovery.rs` cover public bootstrap/rendezvous/relay fallback policy, validation, and source accounting. `qa/tests/observability/observability.rs` verifies public fallback snapshot and metrics fields. `qa/tests/discovery/network_resurrection.rs` verifies that public bootstrap/rendezvous/relay fallback participates only when policy allows it and fallback-only conditions are met.

## DHT provider-discovery checks

`qa/tests/discovery/dht_provider_discovery.rs` covers Kademlia provider-record discovery policy, deterministic namespace record-key derivation, startup/event wiring, and DHT provider snapshot/metric coverage. The test is registered in `Cargo.toml`, so the canonical validation script picks it up automatically.

- `peer_book` verifies merged discovery-source peer metadata for `get_peers()`.
- `network_resurrection` verifies Joe/Alice hashed namespace recovery, hidden raw tags, discovered-but-not-connected peer visibility, and peer-book-fed connection planning.

## Dashboard runtime/exit checks

`qa/tests/observability/dashboard_runtime.rs` guards the standalone `p2p_node` example's full-node and clean-exit invariants: asynchronous terminal events instead of polling, revision-driven redraws without full-snapshot hashing, explicit Full-profile defaults, normal Gossipsub/Ping/DHT cadences, no example-specific 12-connection throttle, five-second deduplicated peer-cache persistence, inbound Kademlia request fast-pathing, DHT/provider observability batching, no connection-triggered DHT refresh feedback loop, no proactive eight-slot DHT disconnect headroom, duplicate Identify observed-address suppression, Windows console-close/logoff/shutdown handling, Unix termination/hangup handling, and the one-second node-task shutdown fail-safe.
