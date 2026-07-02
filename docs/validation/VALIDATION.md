# Stable validation and hostile-network testing

Run the full stable validation script from the crate root.

PowerShell:

```powershell
.\qa\ci\run-full-validation.ps1
```

Bash:

```bash
./qa/ci/run-full-validation.sh
```

The script runs stable validation with DNS enabled by default through p2p-net's own startup resolver. Configured and cached `/dns`, `/dns4`, `/dns6`, and `/dnsaddr` peer addresses are resolved before dialing. Because WebSocket support in rust-libp2p 0.56 expects `libp2p-dns`, p2p-net patches that adapter to a local no-Hickory implementation and patches disallowed mDNS to a local no-op placeholder. Hickory DNS packages are rejected from `Cargo.lock`. `/dnsaddr` uses the configurable bounded DoH policy documented in `docs/impl/DNSADDR_DOH.md`.

The script remains the canonical one-command runner after the profile/environment refactor. Unit tests such as `environment_detection`, `capability_resolver`, `mediator_role`, `event_responsibility`, `behaviour_policy`, `relay_discovery`, `dcutr_policy`, `platform_runtime`, `bindings`, and `codebase_hygiene` are picked up by `cargo test --workspace`, so you do not need separate commands.

The script regenerates the dependency lockfile, auto-formats the tree, runs the dependency graph guard, then runs the stable checks with isolated target directories:

```text
cargo generate-lockfile
cargo fmt
cargo test --workspace --locked -j 1
cargo test --features dashboard --locked -j 1
cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings
cargo audit  # qa/ci/run-full-validation stages qa/ci/audit.toml to .cargo/audit.toml
cargo deny check --config qa/ci/deny.toml
cargo test --test multi_node_hostile --locked -j 1 -- --ignored --nocapture
```

Defaults:

- Missing `cargo-audit` and `cargo-deny` are installed automatically unless `-NoInstallTools` / `--no-install-tools` is used.
- `Cargo.lock` is regenerated first so stale lockfile entries, including old DNS resolver packages, are removed before audit.
- `rust-toolchain.toml` pins the stable toolchain, and the script rejects nightly/beta/dev rustc builds before running validation.
- Fuzz targets are included under `qa/fuzz/`, but they are not run by the stable one-file validation script.

PowerShell options:

```powershell
.\qa\ci\run-full-validation.ps1 -SkipIgnored
.\qa\ci\run-full-validation.ps1 -NoInstallTools
.\qa\ci\run-full-validation.ps1 -NoClean
```

Bash options:

```bash
./qa/ci/run-full-validation.sh --skip-ignored
./qa/ci/run-full-validation.sh --no-install-tools
./qa/ci/run-full-validation.sh --no-clean
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

`qa/tests/relay/relay_discovery.rs` covers relay candidate filtering, source ordering, duplicate removal, policy validation, and lite-profile resolution. The test is registered in `Cargo.toml`, so `qa/ci/run-full-validation.ps1` picks it up through the existing `cargo test --workspace --locked -j 1` step.


## DCUtR policy validation

`qa/tests/relay/dcutr_policy.rs` covers default DCUtR policy safety, disabling the DCUtR capability, rejecting upgrade-without-fallback config, resolved retry policy fields, and snapshot relay-fallback counters. The test is registered in `Cargo.toml`, so `qa/ci/run-full-validation.ps1` picks it up through the existing `cargo test --workspace --locked -j 1` step.

`platform_runtime` is picked up by `cargo test --workspace`.

## Application API validation

`qa/tests/discovery/discovery_namespace.rs` verifies hashed app discovery namespace derivation, duplicate tag removal, readable-debug guardrails, and node-config validation. The test is registered in `Cargo.toml`, so the canonical validation script picks it up automatically.

`qa/tests/api/api_primitives.rs` verifies the six public primitives on `NodeHandle`, app-topic namespacing and validation, `AppMessage` encode/decode behavior, addressed-message filtering, and payload size bounds. The test is registered in `Cargo.toml`, so the canonical validation script picks it up automatically.

## Codebase hygiene validation

`qa/tests/hygiene/codebase_hygiene.rs` guards the cleanup/audit pass by checking that runtime docs do not reintroduce transitional wording, startup does not duplicate profile-to-role decisions, stack builders consume resolved policy instead of user-facing profiles, snapshot JSON derives from `NodeSnapshot` instead of a hand-maintained field list, and every integration test under `qa/tests/` is registered exactly once in `Cargo.toml`.

## Binding facade validation

`qa/tests/api/bindings.rs` covers the cross-platform binding-safe facade:

- the support matrix confirms one shared Rust core and no separate node implementations
- Android runtime specs resolve `profile = auto` to `mobile_lite`
- iOS runtime specs flag memory storage as testing-only
- desktop runtime specs preserve listener capability and desktop filesystem storage
- config and snapshot JSON helpers work for host UI layers

The test is registered in `Cargo.toml`, so `qa/ci/run-full-validation.ps1` and `qa/ci/run-full-validation.sh` pick it up through the existing `cargo test --workspace --locked -j 1` step. Host-language generated binding tests for Kotlin/Swift should live in the app shell once a generator such as UniFFI or a C ABI wrapper is chosen.


## Operator docs and examples checks

`qa/tests/operator/operator_docs.rs` verifies that operator deployment guides exist, consumer-default, private-infrastructure-first, and public-fallback example configs parse as `NodeConfig`, and the examples preserve the expected public-fallback defaults and private-mode override.

## Manual audit and deny commands

`cargo-audit` reads repository audit configuration from `.cargo/audit.toml` in the installed version this project validates against. The canonical validation scripts keep `qa/ci/audit.toml` as the source file and stage it to `.cargo/audit.toml` only while `cargo audit` runs.

`cargo-deny` accepts the config path after the `check` subcommand:

```powershell
cargo deny check --config qa/ci/deny.toml
```

Use `qa/ci/run-full-validation.ps1` or `qa/ci/run-full-validation.sh` for the exact portable flow.

## Public fallback checks

`qa/tests/discovery/discovery.rs` and `qa/tests/relay/relay_discovery.rs` cover public bootstrap/rendezvous/relay fallback policy, validation, and source accounting. `qa/tests/observability/observability.rs` verifies public fallback snapshot and metrics fields. `qa/tests/discovery/network_resurrection.rs` verifies that public bootstrap/rendezvous/relay fallback participates only when policy allows it and fallback-only conditions are met.

## DHT provider-discovery checks

`qa/tests/discovery/dht_provider_discovery.rs` covers Kademlia provider-record discovery policy, deterministic namespace record-key derivation, startup/event wiring, and DHT provider snapshot/metric coverage. The test is registered in `Cargo.toml`, so the canonical validation script picks it up automatically.

- `peer_book` verifies merged discovery-source peer metadata for `get_peers()`.
- `network_resurrection` verifies Joe/Alice hashed namespace recovery, hidden raw tags, discovered-but-not-connected peer visibility, and peer-book-fed connection planning.
