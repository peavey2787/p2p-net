# p2p-net

`p2p-net` is a standalone all-in-one libp2p node crate for peer discovery, signed heartbeat gossip, NAT traversal, DCUtR hole punching, optional Circuit Relay service, optional rendezvous discovery, operator metrics, and hostile-network testing.

## Features

- TCP, QUIC, WebSocket, DNS, Noise, Yamux
- Gossipsub heartbeat mesh with strict/manual validation
- Kademlia, peer cache, bootstrap seeds
- Persistent node identity key and stable `PeerId`
- AutoNAT, relay client, relay reservation, DCUtR upgrade path
- Optional relay server with limits, ACLs, schedule, and abuse telemetry
- First-class DCUtR mediator profile/config mapped onto Circuit Relay v2
- Optional rendezvous client/server discovery layer
- Connection caps, peer/IP limits, replay cache, timestamp checks
- JSON snapshot export and Prometheus-style metrics export
- Dashboard example, security checks, ignored load/soak tests, and fuzz harnesses


DNS support is enabled by default for configured and cached peers through p2p-net's own startup resolver. Peer addresses using `/dns`, `/dns4`, `/dns6`, or `/dnsaddr` are resolved before dialing. WebSocket support in rust-libp2p 0.56 requires the `libp2p-dns` adapter crate, so p2p-net patches that crate to a local no-Hickory implementation instead of using the crates.io resolver path. The unused upstream mDNS adapter crate is policy-patched to a local no-op placeholder so the rejected Hickory DNS line stays out of `Cargo.lock`. `/dnsaddr` uses bounded DNS-over-HTTPS TXT lookup support with a configurable endpoint in p2p-net's own resolver. The default endpoint is Cloudflare for simple out-of-the-box operation; production deployments can point it at an internal/self-hosted DoH resolver or disable `/dnsaddr` entirely. LAN multicast discovery/mDNS is not included.

## Run all stable tests and checks

Use the single full-validation script from the crate root:

```powershell
.\scripts\run-full-validation.ps1
```

It cleans stale build artifacts, refreshes the dependency lockfile, auto-formats with `cargo fmt`, then runs tests, dashboard-feature tests, clippy, `cargo audit`, `cargo deny`, and ignored load/soak tests. It uses isolated validation target directories to avoid stale/incomplete `rlib` artifacts on Windows. It uses the stable Rust toolchain pinned by `rust-toolchain.toml`, rejects nightly/beta/dev rustc builds, and installs missing stable-compatible audit/deny tools unless `-NoInstallTools` is used.

Useful options:

```powershell
.\scripts\run-full-validation.ps1 -SkipIgnored
.\scripts\run-full-validation.ps1 -NoInstallTools
.\scripts\run-full-validation.ps1 -NoClean
```

Linux/macOS equivalent:

```bash
./scripts/run-full-validation.sh
```

Fuzz targets are included under `fuzz/`, but they are not run by the stable one-file validation script. Additional validation and hostile-network notes are in `docs/VALIDATION.md`.

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

## Configure a node

Edit `p2p-node.json`. The default config is also available at:

```text
examples/node-config.example.json
```

Important fields:

- `profile`: high-level node role selection: `auto`, `full`, `lite`, `relay`, `mediator`, `rendezvous`, `bootstrap`, or `mobile_lite`.
- `identity_key_path`: stable private node identity; keep this file private and back it up according to `docs/IDENTITY_KEY_BACKUP_ROTATION.md`.
- `listen_addresses`: local TCP/QUIC/WebSocket listen multiaddrs. Use concrete `/ip4` or `/ip6` listen addresses, not DNS names.
- `bootstrap_peers`: trusted `/p2p/<PeerId>` bootstrap multiaddrs. `/dns`, `/dns4`, `/dns6`, and `/dnsaddr` dial addresses are supported by default.
- `dnsaddr`: `/dnsaddr` DoH policy. Defaults to bounded Cloudflare DoH for simple operation; set `doh_endpoint` to an internal/self-hosted DoH resolver for production, or set `enabled` to `false` to reject `/dnsaddr` in configured peers. See `docs/DNSADDR_DOH.md`.
- `relay_peers`: relay or mediator peers to dial and reserve through.
- `reserve_configured_relays`: request `/p2p-circuit` reservations from configured relays.
- `discovery.rendezvous`: enable rendezvous client/server registration/discovery.
- `connection_limits`: global, per-peer, and per-IP connection caps.
- `message_security`: heartbeat size, timestamp, replay, and reputation settings.
- `mediator`: first-class DCUtR mediator policy. `profile = "mediator"` enables the underlying relay server capability intentionally for lite/mobile peers. See `docs/MEDIATOR.md`.
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

Normally, do not run the individual commands manually. Use `./scripts/run-full-validation.ps1`; it is the canonical one-file validation runner for formatting, tests, clippy, security audit, dependency policy, and ignored load/soak tests.

- `docs/EVENT_HANDLING.md` documents the Phase 5 single-responsibility swarm event split.
