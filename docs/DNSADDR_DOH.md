# `/dnsaddr` DNS-over-HTTPS policy

`/dnsaddr` multiaddrs are resolved by querying TXT records under `_dnsaddr.<domain>` and reading records that start with `dnsaddr=`. For example:

```text
/dnsaddr/bootstrap.example.com/p2p/<PeerId>
```

queries:

```text
_dnsaddr.bootstrap.example.com TXT
```

## Default behavior

DNS support remains enabled by default.

- `/dns`, `/dns4`, and `/dns6` use Tokio's OS resolver.
- `/dnsaddr` uses a bounded DNS-over-HTTPS JSON TXT lookup path.
- The default DoH endpoint is Cloudflare: `https://cloudflare-dns.com/dns-query`.
- Lookup limits are enforced for timeout, recursion depth, record count, total resolved addresses, and TXT value size.
- LAN multicast discovery/mDNS is intentionally not included.

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

The configured `dnsaddr` policy is applied by the startup pre-resolver in `src/connectivity/dns.rs`, which resolves configured and cached peers before dialing. WebSocket support in rust-libp2p 0.56 expects the `libp2p-dns` adapter crate, so p2p-net patches that adapter to a local no-Hickory implementation. The adapter resolves ordinary `/dns`, `/dns4`, and `/dns6` names through Tokio's OS resolver and rejects `/dnsaddr`; `/dnsaddr` is handled only by p2p-net's configurable pre-resolver. The unused upstream mDNS adapter is patched to a local no-op placeholder.

The crate enables libp2p's DNS feature only to satisfy rust-libp2p's WebSocket builder, but the crates.io DNS adapter is replaced with p2p-net's local no-Hickory adapter. `/dnsaddr` is not resolved inside that adapter. The tradeoff is explicit and documented: runtime-discovered `/dnsaddr` multiaddrs are only useful after they are stored and passed back through p2p-net's configured/cache resolution path, or after another discovery layer provides concrete `/ip4` or `/ip6` addresses.

This avoids a split-brain setup where startup resolution uses one resolver but transport-level dialing silently falls back to another.

## Security and privacy notes

- `/dnsaddr` lookups reveal queried `_dnsaddr` domains to the selected DoH provider.
- Use HTTPS endpoints only; plaintext DoH endpoints are rejected by config validation.
- Keep the endpoint bounded and predictable. Do not accept arbitrary untrusted endpoint values from remote peers.
- The resolver accepts only TXT answers that begin with `dnsaddr=` and then parses the contained multiaddr.
- The resolver enforces recursion, count, size, and timeout limits before returning dial candidates.
