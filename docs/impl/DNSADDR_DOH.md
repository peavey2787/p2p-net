# `/dnsaddr` DNS-over-HTTPS policy

`/dnsaddr` multiaddrs are resolved by querying TXT records under `_dnsaddr.<domain>` and reading records that start with `dnsaddr=`. For example:

```text
/dnsaddr/bootstrap.example.com/p2p/<PeerId>
```

queries:

```text
_dnsaddr.bootstrap.example.com TXT
```

When the configured address has a suffix such as `/p2p/<PeerId>`, the resolver
recursively resolves the TXT records and keeps only results ending in that
suffix. It does not append the suffix to TXT results, because DNSADDR records
normally already contain their target peer ID.

## Default behavior

DNS support remains enabled by default.

- `/dns`, `/dns4`, and `/dns6` use Tokio's OS resolver.
- `/dnsaddr` uses a bounded DNS-over-HTTPS JSON TXT lookup path.
- The default DoH endpoint is Cloudflare: `https://cloudflare-dns.com/dns-query`.
- Lookup limits are enforced for timeout, recursion depth, record count, total resolved addresses, and TXT value size.
- Upstream mDNS remains excluded; same-LAN discovery is owned separately by `crates/connectivity/lan.rs` and uses a bounded compatibility-scoped UDP beacon protocol.

This does **not** send the node identity private key. The DoH provider can observe the `_dnsaddr` names queried by the process, so it is still an operational dependency.

## Production configuration

For production deployments that do not want a hard-coded public resolver dependency, configure the DoH endpoint:

```json
{
  "dnsaddr": {
    "enabled": true,
    "doh_endpoint": "https://doh.internal.example/dns-query",
    "timeout_secs": 5
  }
}
```

Recommended production options:

1. Use an internal/self-hosted DoH resolver reachable by the node fleet.
2. Use a deployment-managed public DoH endpoint if an internal resolver is not available.
3. Disable `/dnsaddr` entirely and use concrete `/ip4`, `/ip6`, `/dns`, `/dns4`, or `/dns6` peer addresses.

To disable `/dnsaddr`:

```json
{
  "dnsaddr": {
    "enabled": false
  }
}
```

When disabled, configured `/dnsaddr` peer addresses fail validation with a clear config error. Cached/discovered `/dnsaddr` addresses are ignored during best-effort cache resolution.

## Resolver consistency

The configured `dnsaddr` policy is applied in `crates/connectivity/dns.rs`, which resolves configured, cached, and caller-supplied manual peer addresses before they reach the libp2p transport. Ordinary `/dns`, `/dns4`, and `/dns6` names use Tokio's OS resolver; `/dnsaddr` uses only p2p-net's configurable bounded DoH path. The published crate intentionally leaves rust-libp2p's Hickory-backed DNS and mDNS features disabled, so downstream applications require no Cargo patch or hidden resolver dependency. The source workspace maps Cargo's resolution-only weak DNS/mDNS lock entries to audited no-Hickory placeholders through `.cargo/config.toml`.

The WebSocket transport uses `libp2p-websocket` directly and wraps TCP with p2p-net's own `OsDnsTransport`; it does not enable or depend on `libp2p-dns`, and it does not enable the top-level `libp2p` `websocket` feature whose Tokio builder is coupled to `libp2p-dns`. That adapter resolves ordinary `/dns`, `/dns4`, and `/dns6` names with Tokio's OS resolver and deliberately rejects `/dnsaddr`, which remains owned by p2p-net's configurable bounded pre-resolver. Runtime-discovered `/dnsaddr` multiaddrs are therefore only useful after they are stored and passed back through p2p-net's configured/cache resolution path, or after another discovery layer provides concrete `/ip4` or `/ip6` addresses.

This avoids a split-brain setup where startup resolution uses one resolver but transport-level dialing silently falls back to another.

## Security and privacy notes

- `/dnsaddr` lookups reveal queried `_dnsaddr` domains to the selected DoH provider.
- Use HTTPS endpoints only; plaintext DoH endpoints are rejected by config validation.
- Keep the endpoint bounded and predictable. Do not accept arbitrary untrusted endpoint values from remote peers.
- The resolver accepts only TXT answers that begin with `dnsaddr=` and then parses the contained multiaddr.
- The resolver enforces recursion, count, size, and timeout limits before returning dial candidates.
