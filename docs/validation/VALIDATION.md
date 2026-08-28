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

The script runs stable validation with DNS enabled by default through p2p-net's own resolver. Configured, cached, and manual `/dns`, `/dns4`, `/dns6`, and `/dnsaddr` peer addresses are resolved before dialing. The published crate does not enable rust-libp2p's Hickory-backed DNS or mDNS features and requires no downstream or root-manifest Cargo patch; the source workspace uses only the lock-resolution config patches described below, and Hickory DNS packages remain rejected from the committed `Cargo.lock`. `/dnsaddr` uses the configurable bounded DoH policy documented in `docs/impl/DNSADDR_DOH.md`.

The script remains the canonical one-command runner after the profile/environment refactor. Unit tests such as `environment_detection`, `capability_resolver`, `mediator_role`, `event_responsibility`, `behaviour_policy`, `relay_discovery`, `dcutr_policy`, `platform_runtime`, `bindings`, `dashboard_rendering`, `codebase_hygiene`, and `codebase_architecture_hygiene` are picked up by `cargo test --workspace`, so you do not need separate commands. The dashboard rendering gate also checks narrow/wide bounds, event wrapping, counter separation, and terminal-control/bidi neutralization for peer-derived text.

The production validation path **does not modify dependency or source inputs**. It verifies the committed lockfile, checks formatting, runs the dependency graph guard, and then executes the stable checks in isolated target directories:

```text
cargo metadata --locked --format-version 1
cargo fmt --all -- --check
cargo test --workspace --locked -j 1
cargo test --features dashboard --locked -j 1
cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings
cargo audit --file Cargo.lock  # root launcher stages qa/ci/audit.toml to .cargo/audit.toml
cargo deny --config qa/ci/deny.toml check  # launcher probes equivalent check-level syntax when required
cargo test --test multi_node_hostile --locked -j 1 relay_reservation_spam_does_not_panic -- --ignored --exact --nocapture
cargo test --test multi_node_hostile --locked -j 1 circuit_open_close_spam_does_not_hang -- --ignored --exact --nocapture
cargo test --test multi_node_hostile --locked -j 1 long_running_soak_node_stays_responsive -- --ignored --exact --nocapture
```

Defaults:

- Exact `cargo-audit 0.22.2` and `cargo-deny 0.20.2` releases are installed automatically unless `--no-install-tools` is used.
- `Cargo.lock` is committed and must already match `Cargo.toml`; production validation fails rather than regenerating it.
- `rust-toolchain.toml` pins Rust `1.98.0`, and the launchers fail closed if a different compiler is active.
- On `x86_64-pc-windows-msvc`, `run-full-validation.cmd` locates Visual Studio Build Tools with the installed `vswhere.exe`, initializes `VsDevCmd.bat`, verifies an x64 `ucrt.lib`, and compiles a temporary Rust link smoke test before cleaning or running Cargo validation. If that preflight reports a missing Universal CRT, repair/modify Visual Studio Build Tools and install the **Windows Universal CRT SDK** plus a **Windows 11 SDK** before rerunning.
- Fuzz targets are included under `qa/fuzz/`. The scheduled security workflow uses pinned `nightly-2026-08-20` and `cargo-fuzz 0.13.2`, invoking every fuzz command from the repository root with `--fuzz-dir qa/fuzz` so the nested harness is resolved explicitly.
- GitHub Actions checkout is pinned to an immutable commit SHA and uses read-only repository permissions with credential persistence disabled.

The three `#[ignore]` markers on `multi_node_hostile` are scheduling markers, not omissions. They keep the normal `cargo test --workspace` phase from running the expensive tests twice. The canonical launchers always execute each deferred test explicitly after Clippy/audit/deny, with the one-minute soak test last. The hostile relay and connection-churn tests create their own loopback peers, so they no longer silently return when external environment variables are missing.

Windows CMD options:

```cmd
run-full-validation.cmd --no-install-tools
run-full-validation.cmd --no-clean
run-full-validation.cmd --from clippy
```

Bash options:

```bash
./run-full-validation.sh --no-install-tools
./run-full-validation.sh --no-clean
./run-full-validation.sh --from clippy
```

`--from <stage>` is an explicit local resume mode. Valid stages are `lockfile`, `format`, `dependency-graph`, `tests`, `dashboard`, `clippy`, `audit`, and `deny`. A resumed run still performs the pinned Rust/toolchain preflight, automatically preserves `target/full-validation` instead of cleaning it, starts at the requested validation stage, and then runs every later stage including all three deferred hostile/load/soak tests. It is intended for continuing a validation run after fixing a failure; it assumes all skipped stages already passed for the source tree being continued. CI and reproducible release gates do not use resume mode.

The root validation launchers automatically wrap themselves with the evidence recorder under `qa/evidence/`. Every invocation writes a complete transcript and manifest under `qa/evidence/runs/` before returning its exit code. Evidence generation is not an optional operator step and does not depend on preserving the console window.


Linux packet-loss/latency simulation:

```bash
sudo ./qa/tools/netem-linux.sh lo start
cargo test --test multi_node_hostile -- --ignored --nocapture
sudo ./qa/tools/netem-linux.sh lo stop
```

`netem-linux.sh` requires Linux `tc` and root privileges. Windows should skip netem tests.

## Canonical reproducible release builds

The root `build-release.cmd` and `build-release.sh` launchers are the canonical Windows and Linux release gates. They do not require a clean Git worktree: the runner snapshots the exact current tracked modifications plus non-ignored untracked files into a synthetic detached Git commit, then performs validation and both reproducibility builds from that immutable snapshot. Validation is evidence-backed rather than terminal-backed. A matching durable full-validation record under `qa/evidence/` is reused automatically; if none matches the release-input fingerprint, the complete validation runner executes inside a detached validation worktree from the frozen snapshot located outside the repository tree, then copies its machine-captured evidence back to the ignored evidence store.

Windows:

```cmd
build-release.cmd
```

Linux:

```bash
./build-release.sh
```

Both runners:

- capture the exact current source snapshot with an alternate Git index without changing the user's branch or staging area
- bind `SOURCE_DATE_EPOCH` to the base Git commit timestamp
- verify the exact Rust 1.98.0 toolchain and locked dependency graph
- require a matching full-validation evidence record for the release-input SHA-256, running full validation automatically in a detached snapshot worktree when one is not already available
- fail if validation modifies any tracked file in that frozen snapshot while ignoring disposable untracked validation output for release identity
- create two additional detached clean Git worktrees for the same synthetic source-snapshot commit
- build the production `p2p_node` example with `--release --locked --offline --features dashboard` in two separate clean target directories
- disable incremental compilation and normalize source, target, Cargo-home, and Rustup-home paths through encoded rustflags
- compare the two resulting binaries and fail closed on any SHA-256 mismatch
- write the verified artifact, build manifest, source manifest, release-input manifest, validation evidence, and checksums to `dist/<target-triple>/`

Windows imports the same Visual Studio Build Tools environment used by the validation preflight and requests MSVC `/Brepro` linking. Linux requests a deterministic SHA-1 ELF build ID. These runners prove repeatability across two independent clean source/build directories on the current host. Cross-host byte identity additionally depends on matching native linker, SDK, and system-library inputs, which are recorded operational requirements rather than silently assumed.

`--no-install-tools` is forwarded when a validation run is needed. `--no-pause` keeps both release runners non-interactive for automation. `--force-validation` discards the evidence-reuse optimization and performs a new complete validation run. There is no unaudited skip-validation mode: a release always requires a passing full-validation evidence manifest whose release-input fingerprint matches the code snapshot being built.

Every validation run now writes durable evidence beneath `qa/evidence/runs/` automatically, including the complete transcript, result manifest, Git status, release-input inventory/fingerprint, lockfile hash, toolchain identities, and PASS/FAIL marker. Generated run directories are Git-ignored so evidence collection itself never dirties the source tree. Historical records for runs whose terminal output was not captured may live under `qa/evidence/attestations/`, but they must identify themselves as user-attested and must not fabricate a missing transcript.

### Crates.io package qualification

`package-crates.cmd` and `package-crates.sh` qualify the publishable package boundary without making the companion part of the production workspace lock graph. They first require `cargo metadata --locked` to accept the committed production workspace, package `p2p-net-webrtc`, and then package the root `p2p-net` crate with a command-line-only `[patch.crates-io]` override that maps the unpublished registry name `p2p-net-webrtc` to the local audited companion source. Cargo configuration patches are local build overrides and are not serialized into the normalized crate manifest. The runners therefore inspect both normalized manifests to reject repository-relative dependency `path` entries, `[patch.crates-io]`, a retained workspace table, or embedded companion source, then unpack both `.crate` payloads into a temporary downstream consumer. That consumer independently patches the normalized registry dependency to the packaged companion and compiles the public API. The resulting `.crate` payloads and SHA-256 checksums are written to `dist/crates/`. Publication is deliberately sequential: dry-run and publish `p2p-net-webrtc` first, wait for crates.io to index version 0.1.0, then dry-run and publish `p2p-net`. Downstream applications depend only on `p2p-net = "0.1.0"`; they do not add a patch or companion dependency themselves.

The repository-level `.cargo/config.toml` contains only lock-resolution patches for libp2p 0.56 weak optional DNS/mDNS entries. Those entries point to the audited no-Hickory local placeholders so `cargo metadata --locked` remains deterministic; they are not present in the normalized crates.io manifest and are not required by downstream applications.

### Android reproducible release

`apps/android/build-android.ps1` and `apps/android/build-android.sh` perform two clean release builds with separate Rust target directories, Gradle build/configuration caches disabled, all tasks rerun, incremental Rust compilation disabled, and a fixed `SOURCE_DATE_EPOCH`. The unsigned APK and AAB must match both by SHA-256 and by a direct byte-for-byte comparison. Success writes the verified APK/AAB, `SHA256SUMS.txt`, and `BUILD-MANIFEST.txt` with `apk_byte_identical=true`, `aab_byte_identical=true`, and `reproducible=true` under `dist/android/`. Any mismatch fails closed and retains build-A/build-B artifacts for investigation.

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
The pinned `cargo-audit 0.22.2` output schema is kept explicit in that file (`format`, `quiet`, and `show_tree`) so a partial `[output]` table cannot fail deserialization late in validation.

The canonical launchers pin `cargo-deny 0.20.2` and retain a capability probe for the two CLI layouts seen across supported/local installations:

```text
cargo deny check --config qa/ci/deny.toml
cargo deny --config qa/ci/deny.toml check
```

The launcher probes the check-level form with `--help` and falls back to the global-option form, so local and hosted validation evaluate the same `qa/ci/deny.toml` policy instead of failing on argument placement.

## Intentional dependency refresh

Normal validation never edits `Cargo.lock`. Dependency upgrades are a deliberate maintenance operation:

Windows:

```cmd
qa\tools\refresh-dependencies.cmd
```

Linux/macOS:

```bash
./qa/tools/refresh-dependencies.sh
```

Review the resulting `Cargo.lock` diff and RustSec/license/source changes, then run the full validation launcher before committing the refresh.

## Scheduled security validation

`.github/workflows/security-monthly.yml` repeats the complete validation suite, including the deferred hostile/load/soak tests, and runs bounded libFuzzer campaigns for heartbeat, config, peer cache, application-message, DNSADDR TXT, peer-multiaddr, and WebRTC STUN parsing. Fuzz commands execute from the repository root with `--fuzz-dir qa/fuzz`; using only `working-directory: qa/fuzz` is intentionally forbidden because `cargo-fuzz` otherwise falls back to the root crate's nonexistent `fuzz/Cargo.toml`. The cross-platform push/PR matrix also runs the complete validation launcher; the monthly job adds periodic coverage plus fuzzing rather than relying on skipped tests.

## Public fallback checks

`qa/tests/discovery/discovery.rs` and `qa/tests/relay/relay_discovery.rs` cover public bootstrap/rendezvous/relay fallback policy, validation, and source accounting. `qa/tests/observability/observability.rs` verifies public fallback snapshot and metrics fields. `qa/tests/discovery/network_resurrection.rs` verifies that public bootstrap/rendezvous/relay fallback participates only when policy allows it and fallback-only conditions are met.

## DHT provider-discovery checks

`qa/tests/discovery/dht_provider_discovery.rs` covers Kademlia provider-record discovery policy, deterministic namespace record-key derivation, startup/event wiring, and DHT provider snapshot/metric coverage. The test is registered in `Cargo.toml`, so the canonical validation script picks it up automatically.

- `peer_book` verifies merged discovery-source peer metadata for `get_peers()`.
- `network_resurrection` verifies Joe/Alice hashed namespace recovery, hidden raw tags, discovered-but-not-connected peer visibility, and peer-book-fed connection planning.

## Dashboard runtime/exit checks

`qa/tests/observability/dashboard_runtime.rs` guards the standalone `p2p_node` example's full-node and clean-exit invariants: asynchronous terminal events instead of polling, revision-driven redraws without full-snapshot hashing, explicit Full-profile defaults, normal Gossipsub/Ping/DHT cadences, no example-specific 12-connection throttle, five-second deduplicated peer-cache persistence, inbound Kademlia request fast-pathing, DHT/provider observability batching, no connection-triggered DHT refresh feedback loop, no proactive eight-slot DHT disconnect headroom, duplicate Identify observed-address suppression, Windows console-close/logoff/shutdown handling, Unix termination/hangup handling, and the one-second node-task shutdown fail-safe.
