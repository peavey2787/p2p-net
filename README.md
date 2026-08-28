# p2p-net

`p2p-net` is a Rust libp2p node core that gives applications a small, stable API for peer connectivity and messaging while the crate handles transport mechanics, discovery, relay fallback, DCUtR hole punching, native WebRTC-direct, telemetry, and platform storage/runtime details.

<p align="center">
  <img src="assets/p2p-net-logo.png" alt="p2p-net Logo" width="400">
</p>

On Windows, `build-android.cmd` is the one-click reproducible Android release launcher; it performs the two-build byte-for-byte verification and leaves verified unsigned artifacts under `dist/android/`.

## Features

- Native transports: TCP, QUIC, WebSocket, browser-compatible `/webrtc-direct`, DNS, Noise, Yamux
- Discovery: network-scoped Kademlia provider/address records, bounded same-LAN UDP discovery, peer cache, bootstrap seeds, rendezvous, and public fallback policy
- NAT traversal: relay client/reservations, DCUtR direct upgrades, AutoNAT, and optional mediator/relay server profiles
- App API: six data-plane primitives on `NodeHandle`, plus `get_metrics()` for infrastructure telemetry
- Safety/ops: connection caps, replay/timestamp checks, peer scoring, snapshots, Prometheus-style export, and dashboard UI
- Portability: platform runtime/storage abstraction and binding-safe facade for desktop, mobile, and WebView shells

DNS support is enabled by default through p2p-net's own no-Hickory resolver path. Peer addresses using `/dns`, `/dns4`, `/dns6`, or `/dnsaddr` are resolved before dialing, including manual `connect_peer` calls. WebSocket DNS names are resolved by p2p-net's transport adapter rather than rust-libp2p's Hickory-backed DNS feature, so the published crate has no repository-only Cargo patch requirement. `/dnsaddr` uses bounded DNS-over-HTTPS TXT lookup support with a configurable endpoint; the default is Cloudflare, while production deployments can point it at an internal/self-hosted resolver or disable `/dnsaddr`. Upstream libp2p mDNS is not included. Instead, p2p-net enables its own bounded, compatibility-scoped UDP LAN discovery by default, with multicast/broadcast on normal LANs and an Android Emulator host-assist path using the emulator's `10.0.2.2` host alias.

## Repository layout

```text
crates/              Shared Rust P2P core modules
docs/impl/          Implementation docs
docs/spec/          Core specifications
docs/operator/      Operator deployment guides
docs/validation/    Validation docs
docs/project/       Project/audit notes
docs/future-work/   Deferred ideas and long-term proposals
docs/roadmap.md      Active roadmap, when one exists
qa/ci/              Validation policy/config helpers used by the root launchers
qa/tests/           Domain-grouped global/system/invariant integration tests
qa/fuzz/            Fuzz targets
qa/tools/           Internal QA utilities
qa/vectors/         Protocol fixtures and test vectors
examples/           Runnable examples and minimal demo configs
external/           Publishable hardened companion transport source
```

## Use p2p-net from crates.io

After `p2p-net` 0.1.0 is published, downstream Rust applications only need the root crate:

```toml
[dependencies]
p2p-net = "0.1.0"
```

Then use the high-level async API from your application's existing executor:

```rust
use p2p_net::{start_node, NetError, NodeConfig};

async fn run_node() -> Result<(), NetError> {
    let node = start_node(NodeConfig::default()).await?;
    let peers = node.get_peers().await?;
    println!("known peers: {}", peers.len());
    node.shutdown().await;
    Ok(())
}
```

The hardened WebRTC implementation is published as the internal dependency `p2p-net-webrtc` 0.1.0. Application developers do **not** add that crate themselves; Cargo resolves it automatically from the `p2p-net` dependency graph. The repository keeps a local path to that companion only for development, and Cargo strips that path from the normalized crates.io package. There is no `[patch.crates-io]` requirement in downstream projects. The source workspace uses a checked-in `.cargo/config.toml` only to map rust-libp2p 0.56's resolution-only weak DNS/mDNS lock entries to audited no-Hickory local placeholders; the root publishable manifest remains patch-free, and the packaged downstream smoke test runs outside the repository configuration.

Maintainers can qualify the exact crates.io payloads with `package-crates.cmd` on Windows or `./package-crates.sh` on Linux. The Windows launcher pauses before closing on both success and failure so Cargo diagnostics remain visible. The packaging gate first verifies the committed production lockfile, packages `p2p-net-webrtc`, then packages `p2p-net` with a command-line-only crates.io patch that points the unpublished companion name at the local audited source. That local verification override is not serialized into the normalized `.crate`; the runners inspect the normalized manifests, compile a temporary downstream consumer from both packaged payloads, write `.crate` files and SHA-256 sums to `dist/crates/`, and record the required publish order. Publish `p2p-net-webrtc` first, wait until crates.io indexes version 0.1.0, then dry-run and publish `p2p-net`.

## Run all stable tests and checks

Use the single full-validation launcher for your OS from the crate root. On Windows, double-click `run-full-validation.cmd`; on Linux, `run-full-validation.sh` is executable and can be launched directly from a file manager/terminal:

```cmd
run-full-validation.cmd
```

It cleans stale build artifacts, verifies the committed dependency lockfile with `--locked`, checks formatting without mutating source, then runs tests, dashboard-feature tests, clippy, `cargo audit`, and `cargo deny`. Three intentionally long hostile/load/soak tests are deferred so they run once at the end, with the one-minute soak test last. The full runner has no skip option for those tests: `run-full-validation` means all registered tests run. It uses isolated validation target directories to avoid stale/incomplete `rlib` artifacts on Windows. Rust is pinned to 1.98.0, audit/deny tool releases are pinned, and missing exact tool versions are installed unless `--no-install-tools` is used.

On an MSVC Rust host, the Windows launcher also initializes the installed Visual Studio C++ developer environment before Cargo can link anything. It verifies that the x64 Universal CRT (`ucrt.lib`) is installed and performs a tiny `rustc` link smoke test up front, so an incomplete Build Tools/Windows SDK installation fails immediately with an actionable preflight error instead of surfacing halfway through Clippy.

Useful options:

```cmd
run-full-validation.cmd --no-install-tools
run-full-validation.cmd --no-clean
run-full-validation.cmd --from clippy
```

Linux equivalent:

```bash
./run-full-validation.sh
./run-full-validation.sh --from clippy
```

Resume mode accepts `lockfile`, `format`, `dependency-graph`, `tests`, `dashboard`, `clippy`, `audit`, or `deny`. For example, after fixing a Clippy-only failure, `--from clippy` skips the already-passed earlier stages, preserves `target/full-validation` automatically (equivalent to `--no-clean`), and continues from Clippy through audit, dependency policy, and all three deferred hostile/soak tests. Resume mode assumes the earlier stages already passed for the source tree you are continuing; CI never uses resume mode, and release builds only accept full-mode evidence (running the complete gate automatically when matching full evidence is absent).

Every validation invocation now persists its own evidence beneath `qa/evidence/runs/`: a complete transcript, PASS/FAIL manifest, Git status, exact Cargo.lock hash, toolchain versions, and a release-input fingerprint. The generated run directories are Git-ignored, so closing the terminal no longer loses the proof and collecting evidence does not dirty the repository.

Fuzz targets are included under `qa/fuzz/`. They are not part of the cross-platform stable launcher, but the scheduled security workflow builds/runs every fuzz target and also repeats the complete validation suite including deferred hostile/load/soak tests. Additional validation and hostile-network notes are in `docs/validation/VALIDATION.md`.


## The General-Purpose Application API

Applications should build on our stable, high-level API surface instead of depending on low-level swarm or libp2p internals. Every application builds on the same six primitives exposed by `NodeHandle`:

```rust
// Establish connection to a target address
handle.connect_peer(addr).await?;

// Drop connection to a peer
handle.disconnect_peer(peer_id).await?;

// Addressed delivery to a specific peer. The carrier is gossipsub, so encrypt
// payloads end-to-end when confidentiality is required.
handle.send_message(peer_id, "chat/general", payload).await?;

// Gossip/PubSub broadcast to all subscribed peers on a topic
handle.broadcast("game/lobby", payload).await?;

// Subscribe to a topic and receive a local message stream
let mut messages = handle.subscribe("chat/general").await?;

// Query known peers and discovery metrics
let peers = handle.get_peers().await?;
```

This simple interface effectively decouples your business logic (chat, gaming, database sync) from transport mechanics (TCP, WebRTC, QUIC, NAT-punting).

Application messages use `AppMessage` envelopes and app topics namespaced as `p2p-net/app/v2/net-<network_id>/<topic>`. See `docs/spec/API_PRIMITIVES.md` and `docs/impl/API_IMPLEMENTATION.md`.

### Telemetry without payment logic

The core primitives stay free of wallet, token, and payment-settlement code. Applications that need usage accounting can build those layers above the node by calling the seventh query/management primitive:

```rust
let all_metrics = handle.get_metrics(None).await?;
let peer_metrics = handle.get_metrics(Some(peer_id)).await?;
```

`get_metrics()` reports runtime-owned bandwidth, storage, and compute counters, including per-peer and per-topic bandwidth where available. That gives apps enough low-level data to implement custom settlement, micropayment, quota, or billing systems without coupling financial logic into the networking core.

## Start a node

Generate the example's full-node config:

```powershell
cargo run --release --features dashboard --example p2p_node -- --write-default-config p2p-node.json
```

Run with the config:

```powershell
cargo run --release --features dashboard --example p2p_node -- --config p2p-node.json
```

The standalone example is deliberately production-shaped rather than throttled: it selects the `full` profile, runs a Kademlia server, keeps all configured inbound transports available, uses the normal 5-second Gossipsub heartbeat and 15-second Ping cadence, uses the normal three-way DHT query parallelism/provider-key replication, and retains the standard production connection-safety policy. Developers can copy the generated JSON and selectively change roles, listeners, limits, or discovery policy for their deployment. Runtime optimizations are implemented in hot paths instead of by silently reducing protocol capability: public Kademlia request events bypass app-side bookkeeping, DHT/provider observability is coalesced, duplicate Identify/public-address work is suppressed, peer-cache writes are deduplicated and batched, and recovery refreshes cannot form a discovery-to-connection feedback loop. Native WebRTC-direct also bounds and expires unverified pre-handshake UDP state, cleans failed/cancelled ICE mux connections, and closes peer connections on drop so public listener churn cannot retain transport state indefinitely.

The dashboard stays on the existing `crossterm` backend and does not pull `ratatui` or `lru` back into the dependency graph. Its full-screen view uses responsive color-coded panels for reachability, peer state, discovery/relay/DHT health, traffic counters, and a newest-first event stream. Event text is wrapped inside the panel instead of spilling across the console. All dynamic terminal text is bounded and normalized before rendering so peer-derived control characters, escape sequences, bidi controls, and other non-ASCII formatting cannot become terminal commands or deceptive layout.

Press `q` or `Esc`, use Ctrl-C, or close the console window to stop the dashboard node cleanly. Windows console close/logoff/shutdown and Unix SIGTERM/SIGHUP are handled, and node shutdown has a one-second fail-safe before its runtime task is aborted.

Run and ship optimized release builds. Debug-mode libp2p/crypto/network code is substantially more CPU-intensive and is intended for development diagnostics only.

### Canonical reproducible release builds

After the full validation gate is green, use the root release runner for the host platform:

```cmd
build-release.cmd
```

```bash
./build-release.sh
```

The release runners snapshot the exact current Git working tree into a synthetic detached commit, including tracked modifications and non-ignored untracked files, so an official reproducibility build does not require committing an otherwise validated source snapshot first. They bind the release to a durable full-validation evidence record whose release-input fingerprint matches the code being built; if no matching evidence exists, they validate a detached worktree created from that frozen snapshot outside the repository tree and copy the resulting machine evidence back to the ignored evidence store. Validation fails if it changes any tracked snapshot file, while generated/untracked validation state cannot redefine the already-frozen release identity. `--force-validation` intentionally reruns validation even when matching evidence exists. The exact snapshot is then built twice from two independent detached clean worktrees with separate target directories, build paths are normalized, incremental compilation is disabled, and the build fails unless the two release binaries have identical SHA-256 output. Windows enables MSVC `/Brepro`; Linux uses a deterministic SHA-1 ELF build ID. The output under `dist/<target-triple>/` includes the binary, `BUILD-MANIFEST.txt`, `SHA256SUMS.txt`, `SOURCE-MANIFEST.txt`, `RELEASE-INPUTS.txt`, and the complete `validation-evidence/` bundle used for the release.

## Default connectivity model

Normal app mode uses public fallback by default:

```text
fresh node -> LAN discovery and public bootstrap/DHT discovery run in parallel -> dialable app peers are recovered -> direct/relay auto-connect attempts start -> DCUtR upgrades relayed paths when possible -> contact trust still requires invite/QR/join-code
```

Regular users should not have to edit bootstrap settings before first launch. Manual `bootstrap_peers`, `discovery.bootstrap_seed_peers`, `discovery.rendezvous_peers`, and `relay_peers` are power-user/operator controls and should be exposed under Advanced settings in app UIs.

Auto-connect is **not** auto-trust. A peer discovered through public fallback, DHT provider records, or rendezvous may be dialed at the transport layer, but it must not become a trusted chat/contact identity until the app performs an explicit trust action such as QR scan, join code, invite acceptance, or safety-number verification.

The shared crate ships public bootstrap defaults but does not depend on a project-operated rendezvous or relay fleet. Fresh default nodes use a network-specific application namespace, publish/query DHT provider records, recover identity-signed dialable-address records, discover public Circuit Relay v2 hop peers from the public DHT, reserve relay routes when needed, and attempt DCUtR upgrades while retaining relay fallback. Nodes on the same LAN take the faster bounded UDP discovery path first. Operator-provided rendezvous/relay peers remain supported as additional deterministic infrastructure, but they are no longer required for the default discovery path to exchange application-peer addresses.

Runtime snapshots and Prometheus metrics report public fallback by category: `public_bootstrap_used`, `public_rendezvous_used`, and `public_relay_used`. Peer metadata also distinguishes `public_rendezvous` from operator-provided `rendezvous` sources.

Private-infrastructure-first operation is still supported as an Advanced/operator mode by setting `discovery.public_bootstrap.mode` to `disabled` and configuring owned bootstrap/rendezvous/relay infrastructure explicitly. See `docs/operator/CONSUMER_DEFAULT.md` for the normal app walkthrough and `docs/operator/PRIVATE_INFRASTRUCTURE_FIRST.md` for the private mode override.

## Configure a node

Edit `p2p-node.json`. The default config is also available at:

```text
examples/node-config.example.json
```

Example configs are also available:

```text
examples/consumer-default.config.json
examples/public-fallback.config.json
examples/private-infrastructure-first.config.json
```

Use `examples/consumer-default.config.json` for normal app mode. `examples/public-fallback.config.json` is an expanded public-fallback example for operators and power users. `examples/private-infrastructure-first.config.json` is the Advanced/operator private mode.

Operator guidance is in `docs/operator/`.

Important fields:

- `profile`: high-level node role selection: `auto`, `full`, `lite`, `relay`, `mediator`, `rendezvous`, `bootstrap`, or `mobile_lite`.
- `identity_key_path`: stable private node identity; keep this file private and back it up according to `docs/spec/IDENTITY_KEY_BACKUP_ROTATION.md`.
- `listen_addresses`: local TCP/QUIC/WebSocket/WebRTC-direct listen multiaddrs. Use concrete `/ip4` or `/ip6` listen addresses, not DNS names.
- `listeners`: per-transport inbound listener switches. Set `websocket` and/or `webrtc_direct` to `false` when those inbound transports are not needed; outbound dial support remains available.
- `bootstrap_peers`: trusted `/p2p/<PeerId>` bootstrap multiaddrs. `/dns`, `/dns4`, `/dns6`, and `/dnsaddr` dial addresses are supported by default.
- `dnsaddr`: `/dnsaddr` DoH policy. Defaults to bounded Cloudflare DoH for simple operation; set `doh_endpoint` to an internal/self-hosted DoH resolver for production, or set `enabled` to `false` to reject `/dnsaddr` in configured peers. See `docs/impl/DNSADDR_DOH.md`.
- `relay_peers`: operator-pinned relay or mediator peers to dial and reserve through.
- `discovery.namespace`: derive hashed app/contact/group discovery namespaces from `network_id`, `app_id`, and tags. With no explicit tags, the built-in default derives a reserved network-specific application namespace instead of publishing one generic `p2p-net` provider key. See `docs/spec/DISCOVERY_NAMESPACES.md`.
- `discovery.lan`: bounded same-LAN multicast/broadcast discovery. Enabled by default; discovery hints are compatibility-scoped and the resulting libp2p connection is still authenticated by Noise + Identify.
- `discovery.public_bootstrap`: public bootstrap, rendezvous, relay, and network auto-connect fallback. Defaults to `fallback_only` for normal app mode; set `disabled` for private-infrastructure-first mode or `always` for aggressive public fallback. See `docs/spec/PUBLIC_FALLBACK.md`.
- `discovery.dht`: announce and query hashed app namespaces through Kademlia provider records. See `docs/spec/DHT_PROVIDER_DISCOVERY.md`.
- `discovery.relay_discovery`: select relay candidates from configured relays, cached healthy peers, rendezvous infrastructure, and public-DHT peers that advertise Circuit Relay v2 hop capability. See `docs/impl/RELAY_DISCOVERY.md`.
- `dcutr`: direct-connection upgrade policy with relay fallback, retry budget, and observability. See `docs/impl/DCUTR_POLICY.md`.
- `start_node_with_platform(...)`: embed the shared core with platform runtime/storage adapters. See `docs/impl/PLATFORM_RUNTIME.md`.
- `bindings`: binding-safe JSON/enum facade for Kotlin, Swift, desktop, and WebView shells. See `docs/impl/BINDINGS.md`.
- `NodeHandle`: six app data-plane primitives plus `get_metrics()` for runtime-owned telemetry. See `docs/spec/API_PRIMITIVES.md`.
- `reserve_configured_relays`: request `/p2p-circuit` reservations from selected relays.
- `discovery.rendezvous`: enable rendezvous client/server registration/discovery.
- `connection_limits`: global, per-peer, and per-IP connection caps.
- `message_security`: heartbeat size, timestamp, replay, and reputation settings.
- Heartbeat gossip uses the compact binary `p2p-net/heartbeat/v2` wire format; the former JSON heartbeat wire is intentionally version-separated.
- `mediator`: first-class DCUtR mediator policy. `profile = "mediator"` enables the underlying relay server capability intentionally for lite/mobile peers. See `docs/impl/MEDIATOR.md`.
- `relay.enabled`: opt-in generic relay server mode. Defaults to `false`.
- `relay.allow_peers` / `relay.deny_peers`: connection-level relay-node ACLs; deny wins.
- `relay.schedule`: UTC relay service windows.

Minimal volunteer relay config:

```json
{
  "relay": {
    "enabled": true
  }
}
```

Minimal DCUtR mediator config:

```json
{
  "profile": "mediator"
}
```

Relay/mediator ACL note: current ACL enforcement is connection-level. A peer denied from relay/mediator use is denied from connecting to that node at all.

## Production-readiness status

The shared core and full-node example are hardened for production use when the repository release gates are green: exact Rust/toolchain inputs, committed `Cargo.lock`, tests/clippy/audit/deny, hostile/soak coverage, and scheduled fuzzing. Network-facing state is bounded, identity persistence is fail-closed, application envelopes are authenticated against signed gossipsub authors/topics, and the full node keeps Kademlia/relay/WebRTC capabilities enabled by default.

Production operators still own deployment concerns that no library can supply automatically: monitoring/alerting, identity backup/rotation, firewall/NAT policy, capacity planning, secure application payload design, and representative multi-host soak testing. External security review is strongly recommended for high-value deployments, but is not substituted by or falsely implied by the repository's automated gates.

## Manual checks

Normally, do not run the individual commands manually. Use `run-full-validation.cmd` on Windows or `./run-full-validation.sh` on Linux; these are the canonical root-level one-file validation runners for formatting, tests, clippy, security audit, dependency policy, and the deferred hostile/load/soak tests that are always executed at the end.

- `docs/impl/EVENT_HANDLING.md` documents the single-responsibility swarm event split.
- `docs/impl/BEHAVIOUR_POLICY.md` documents profile-driven behaviour construction.
- `docs/impl/RELAY_DISCOVERY.md` documents relay discovery and selection.
- `docs/impl/DCUTR_POLICY.md` documents DCUtR policy and fallback counters.
- `docs/impl/PLATFORM_RUNTIME.md` documents the platform runtime/storage abstraction.
- `docs/impl/BINDINGS.md` documents the cross-platform binding facade.
- `docs/spec/API_PRIMITIVES.md` documents the application API and telemetry query primitive.
- `docs/spec/DISCOVERY_RESURRECTION.md` documents consumer-default public fallback, advanced private-infrastructure mode, and peer roles.
- `docs/spec/DISCOVERY_NAMESPACES.md` documents hashed app discovery namespace derivation.
- `docs/spec/PUBLIC_FALLBACK.md` documents default public bootstrap, rendezvous, relay, and auto-connect fallback policy.
- `docs/operator/README.md` links deployment examples and production operator guidance.
- `docs/spec/DHT_PROVIDER_DISCOVERY.md` documents DHT provider-record namespace discovery.
- `docs/spec/PEER_BOOK.md` documents normalized peer metadata returned by `get_peers()`.
- `docs/impl/DHT_PROVIDER_DISCOVERY_IMPLEMENTATION.md` documents DHT provider wiring and observability.
- `docs/impl/PEER_BOOK_IMPLEMENTATION.md` documents peer-book update paths and observability.
- `docs/impl/PUBLIC_FALLBACK_IMPLEMENTATION.md` documents the startup wiring for public fallback.
- `docs/impl/API_IMPLEMENTATION.md` documents API command routing and message delivery.
- `qa/tests/hygiene/codebase_hygiene.rs` guards repository/layout and profile-decision hygiene, while `qa/tests/hygiene/codebase_architecture_hygiene.rs` guards focused module ownership and complete, unique test registration.
