# p2p-net

`p2p-net` is a shared libp2p node core with six general-purpose application primitives: `connect_peer`, `disconnect_peer`, `send_message`, `broadcast`, `subscribe`, and `get_peers`. It also provides peer discovery, signed heartbeat gossip, NAT traversal, DCUtR hole punching, relay mediation, rendezvous discovery, operator metrics, platform adapters, binding-safe helpers, and hostile-network testing.

## Features

- TCP, QUIC, WebSocket, DNS, Noise, Yamux
- Gossipsub heartbeat mesh with strict/manual validation
- Kademlia, peer cache, bootstrap seeds, and default public fallback policy
- Persistent node identity key and stable `PeerId`
- AutoNAT, relay client, relay reservation, DCUtR upgrade path
- Optional relay server with limits, ACLs, schedule, and abuse telemetry
- First-class DCUtR mediator profile/config mapped onto Circuit Relay v2
- Optional rendezvous client/server discovery layer
- Connection caps, peer/IP limits, replay cache, timestamp checks
- JSON snapshot export and Prometheus-style metrics export
- Dashboard example, security checks, ignored load/soak tests, and fuzz harnesses
- Profile-driven libp2p behaviour policy, including Kademlia client mode for lite/mobile nodes
- Relay discovery and selection from configured, cached, and rendezvous candidate sources
- Explicit DCUtR policy with relay fallback, retry controls, and counters
- Platform runtime/storage abstraction for desktop and mobile adapters
- Binding-safe facade for desktop, Android, iOS/iPadOS, and WASM/WebView shells
- Six stable application primitives exposed on `NodeHandle`: `connect_peer`, `disconnect_peer`, `send_message`, `broadcast`, `subscribe`, and `get_peers`
- Consumer-default public fallback with advanced private-infrastructure-first override
- Hashed application discovery namespaces for contact/group tags without raw tag publication
- Kademlia provider-record discovery for hashed namespaces when rendezvous is unavailable

DNS support is enabled by default for configured and cached peers through p2p-net's own startup resolver. Peer addresses using `/dns`, `/dns4`, `/dns6`, or `/dnsaddr` are resolved before dialing. WebSocket support in rust-libp2p 0.56 requires the `libp2p-dns` adapter crate, so p2p-net patches that crate to a local no-Hickory implementation instead of using the crates.io resolver path. The disallowed upstream mDNS adapter crate is policy-patched to a local no-op placeholder so the rejected Hickory DNS line stays out of `Cargo.lock`. `/dnsaddr` uses bounded DNS-over-HTTPS TXT lookup support with a configurable endpoint in p2p-net's own resolver. The default endpoint is Cloudflare for simple out-of-the-box operation; production deployments can point it at an internal/self-hosted DoH resolver or disable `/dnsaddr` entirely. LAN multicast discovery/mDNS is not included.


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
qa/ci/              Canonical validation scripts and CI helpers
qa/tests/           Domain-grouped global/system/invariant integration tests
qa/fuzz/            Fuzz targets
qa/tools/           Internal QA utilities
qa/vectors/         Protocol fixtures and test vectors
examples/           Runnable examples and minimal demo configs
external/           Local third-party crate patches
```

## Run all stable tests and checks

Use the single full-validation script from the crate root:

```powershell
.\qa\ci\run-full-validation.ps1
```

It cleans stale build artifacts, refreshes the dependency lockfile, auto-formats with `cargo fmt`, then runs tests, dashboard-feature tests, clippy, `cargo audit`, `cargo deny`, and ignored load/soak tests. It uses isolated validation target directories to avoid stale/incomplete `rlib` artifacts on Windows. It uses the stable Rust toolchain pinned by `rust-toolchain.toml`, rejects nightly/beta/dev rustc builds, and installs missing stable-compatible audit/deny tools unless `-NoInstallTools` is used.

Useful options:

```powershell
.\qa\ci\run-full-validation.ps1 -SkipIgnored
.\qa\ci\run-full-validation.ps1 -NoInstallTools
.\qa\ci\run-full-validation.ps1 -NoClean
```

Linux/macOS equivalent:

```bash
./qa/ci/run-full-validation.sh
```

Fuzz targets are included under `qa/fuzz/`, but they are not run by the stable one-file validation script. Additional validation and hostile-network notes are in `docs/validation/VALIDATION.md`.


## General-purpose application API

Every application builds on the same six primitives exposed by `NodeHandle`:

```rust
handle.connect_peer(addr).await?;
handle.disconnect_peer(peer_id).await?;
handle.send_message(peer_id, "chat/general", payload).await?;
handle.broadcast("game/lobby", payload).await?;
let mut messages = handle.subscribe("chat/general").await?;
let peers = handle.get_peers().await?;
```

Application messages use `AppMessage` envelopes and app topics namespaced as `p2p-net/app/v1/net-<network_id>/<topic>`. See `docs/spec/API_PRIMITIVES.md` and `docs/impl/API_IMPLEMENTATION.md`.

## Start a node

Generate a config:

```powershell
cargo run --features dashboard --example p2p_node -- --write-default-config p2p-node.json
```

Run with the config:

```powershell
cargo run --features dashboard --example p2p_node -- --config p2p-node.json
```

Press `q` or `Esc` to stop the dashboard node cleanly.

## Default connectivity model

Normal app mode uses public fallback by default:

```text
fresh node -> owned/cached peers if present -> built-in public bootstrap fallback -> DHT/rendezvous/relay discovery paths
```

That means regular users should not have to edit bootstrap settings before first launch. Manual `bootstrap_peers`, `discovery.bootstrap_seed_peers`, `discovery.rendezvous_peers`, and `relay_peers` are power-user/operator controls and should be exposed under Advanced settings in app UIs.

The shared crate ships public bootstrap defaults, but it does not ship a project-operated public relay fleet. Apps that need reliable NAT-to-NAT first-launch connectivity should add real public relay/mediator DNSADDR entries under `discovery.public_bootstrap.relay_peers` or operate private relays.

Private-infrastructure-first operation is still supported by setting `discovery.public_bootstrap.mode` to `disabled` and configuring owned bootstrap/rendezvous/relay infrastructure explicitly.

## Configure a node

Edit `p2p-node.json`. The default config is also available at:

```text
examples/node-config.example.json
```

Operator-oriented configs are also available:

```text
examples/public-fallback.config.json
examples/private-infrastructure-first.config.json
```

Operator guidance is in `docs/operator/`.

Important fields:

- `profile`: high-level node role selection: `auto`, `full`, `lite`, `relay`, `mediator`, `rendezvous`, `bootstrap`, or `mobile_lite`.
- `identity_key_path`: stable private node identity; keep this file private and back it up according to `docs/spec/IDENTITY_KEY_BACKUP_ROTATION.md`.
- `listen_addresses`: local TCP/QUIC/WebSocket listen multiaddrs. Use concrete `/ip4` or `/ip6` listen addresses, not DNS names.
- `bootstrap_peers`: trusted `/p2p/<PeerId>` bootstrap multiaddrs. `/dns`, `/dns4`, `/dns6`, and `/dnsaddr` dial addresses are supported by default.
- `dnsaddr`: `/dnsaddr` DoH policy. Defaults to bounded Cloudflare DoH for simple operation; set `doh_endpoint` to an internal/self-hosted DoH resolver for production, or set `enabled` to `false` to reject `/dnsaddr` in configured peers. See `docs/impl/DNSADDR_DOH.md`.
- `relay_peers`: operator-pinned relay or mediator peers to dial and reserve through.
- `discovery.namespace`: derive hashed app/contact/group discovery namespaces from `network_id`, `app_id`, and tags. See `docs/spec/DISCOVERY_NAMESPACES.md`.
- `discovery.public_bootstrap`: public bootstrap and relay fallback. Defaults to `fallback_only` for normal app mode; set `disabled` for private-infrastructure-first mode or `always` for aggressive public fallback. See `docs/spec/PUBLIC_FALLBACK.md`.
- `discovery.dht`: announce and query hashed app namespaces through Kademlia provider records. See `docs/spec/DHT_PROVIDER_DISCOVERY.md`.
- `discovery.relay_discovery`: select relay candidates from configured relays, cached healthy peers, and rendezvous infrastructure. See `docs/impl/RELAY_DISCOVERY.md`.
- `dcutr`: direct-connection upgrade policy with relay fallback, retry budget, and observability. See `docs/impl/DCUTR_POLICY.md`.
- `start_node_with_platform(...)`: embed the shared core with platform runtime/storage adapters. See `docs/impl/PLATFORM_RUNTIME.md`.
- `bindings`: binding-safe JSON/enum facade for Kotlin, Swift, desktop, and WebView shells. See `docs/impl/BINDINGS.md`.
- `NodeHandle`: six app primitives for connecting peers, sending/broadcasting payloads, subscribing to topics, and listing peers. See `docs/spec/API_PRIMITIVES.md`.
- `reserve_configured_relays`: request `/p2p-circuit` reservations from selected relays.
- `discovery.rendezvous`: enable rendezvous client/server registration/discovery.
- `connection_limits`: global, per-peer, and per-IP connection caps.
- `message_security`: heartbeat size, timestamp, replay, and reputation settings.
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

The crate is intended to be validation-clean under the stable validation script and suitable as a hardened standalone libp2p node foundation. Identity key backup/rotation and `/dnsaddr` DoH provider configuration are documented. Before treating a deployment as production service infrastructure, finish deployment-specific work outside this crate: persistent operational monitoring/alerting, externally reviewed relay abuse policy, real multi-host soak tests under representative NAT/firewall conditions, and an independent security review of the DNS resolution and relay policy.

## Manual checks

Normally, do not run the individual commands manually. Use `.\qa\ci\run-full-validation.ps1`; it is the canonical one-file validation runner for formatting, tests, clippy, security audit, dependency policy, and ignored load/soak tests.

- `docs/impl/EVENT_HANDLING.md` documents the single-responsibility swarm event split.
- `docs/impl/BEHAVIOUR_POLICY.md` documents profile-driven behaviour construction.
- `docs/impl/RELAY_DISCOVERY.md` documents relay discovery and selection.
- `docs/impl/DCUTR_POLICY.md` documents DCUtR policy and fallback counters.
- `docs/impl/PLATFORM_RUNTIME.md` documents the platform runtime/storage abstraction.
- `docs/impl/BINDINGS.md` documents the cross-platform binding facade.
- `docs/spec/API_PRIMITIVES.md` documents the six primitive application API.
- `docs/spec/DISCOVERY_RESURRECTION.md` documents consumer-default public fallback, advanced private-infrastructure mode, and peer roles.
- `docs/spec/DISCOVERY_NAMESPACES.md` documents hashed app discovery namespace derivation.
- `docs/spec/PUBLIC_FALLBACK.md` documents default public bootstrap and relay fallback policy.
- `docs/operator/README.md` links deployment examples and production operator guidance.
- `docs/spec/DHT_PROVIDER_DISCOVERY.md` documents DHT provider-record namespace discovery.
- `docs/spec/PEER_BOOK.md` documents normalized peer metadata returned by `get_peers()`.
- `docs/impl/DHT_PROVIDER_DISCOVERY_IMPLEMENTATION.md` documents DHT provider wiring and observability.
- `docs/impl/PEER_BOOK_IMPLEMENTATION.md` documents peer-book update paths and observability.
- `docs/impl/PUBLIC_FALLBACK_IMPLEMENTATION.md` documents the startup wiring for public fallback.
- `docs/impl/API_IMPLEMENTATION.md` documents API command routing and message delivery.
- `qa/tests/hygiene/codebase_hygiene.rs` guards against stale transitional docs, grouped test registration, and profile-decision drift outside the resolver.
