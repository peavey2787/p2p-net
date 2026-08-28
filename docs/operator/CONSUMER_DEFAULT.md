# Consumer default public-network mode

Consumer default mode is the normal app experience. A regular user should be able to install the app, run it, and join the public app network without editing `bootstrap_peers` or understanding libp2p multiaddrs.

The default mode is not private-infrastructure-first. Private infrastructure remains available under Advanced/operator settings.

## First-run flow

The intended first-run network flow is:

1. The app starts with `discovery.public_bootstrap.mode = "fallback_only"`.
2. If the user has no manual bootstrap peers, no owned rendezvous peers, and no useful peer cache yet, the node uses bundled public bootstrap candidates.
3. If the app distribution supplies public rendezvous peers under `discovery.public_bootstrap.rendezvous_peers`, the node registers and discovers app-namespace peers through those public rendezvous servers.
4. The node also announces and queries hashed app namespaces through DHT provider discovery.
5. If `auto_connect_discovered_peers = true`, discovered app peers are dialed at the network layer through the shared connection strategy planner.
6. Direct QUIC/TCP addresses are tried before relayed paths.
7. If the app distribution supplies public relay or mediator peers under `discovery.public_bootstrap.relay_peers`, NATed nodes can reserve relay paths and later attempt DCUtR direct upgrade.
8. Peers that connect through this flow are still not trusted contacts.

The simple product-level summary is:

```text
run app -> public fallback joins discovery -> app peers are discovered -> network auto-connect attempts start -> contact trust still requires invite/QR/join-code
```

Runtime status separates those stages. `PeerBook: known/discovered` means the node has learned about peers. `Auto-Connect` shows whether the runtime has started dial attempts, still has pending connection plans, or is waiting for dialable addresses. `Connected Peers` only counts established libp2p connections.

`NAT/Public` only shows confirmed public direct or relayed reachability. Local/private listen addresses such as `127.0.0.1`, `172.17.0.1`, RFC1918 LAN ranges, and CGNAT ranges are shown separately as `Local Listen` diagnostics and are not advertised as public addresses. Normal consumer mode also enables `public_ip_probe` by default, which queries HTTPS public-IP endpoints and synthesizes concrete external multiaddrs from the configured listen ports. AutoNAT status `unknown_no_public_direct_addr_yet` means AutoNAT itself has no public address to test yet; it does not disable the public-IP probe or relay fallback.

## Auto-connect is not auto-trust

Consumer default mode may connect to discovered peers so the transport mesh can form. That must never mean the peer becomes a trusted chat/contact identity.

A peer should become a trusted contact only after an explicit app-level action such as:

- scanning a QR code;
- entering a join code;
- accepting an invite;
- verifying a safety number;
- importing a trusted contact card.

This separation allows the network layer to be easy for normal users while keeping contact trust intentional.

## Default config shape

Use `examples/consumer-default.config.json` as the consumer-facing example. Its important properties are:

- the example uses the `full` profile and serves Kademlia while retaining DHT client discovery;
- TCP, QUIC, WebSocket, and WebRTC-direct inbound listeners remain enabled by default;
- iterative DHT query parallelism remains 3, provider-key replication remains 3, and the normal 300-second periodic bootstrap timer remains enabled;
- Gossipsub mesh maintenance remains at 5 seconds and connection pings remain at 15 seconds;
- rendezvous discovery keeps its normal 64-peer batch and relay discovery keeps its normal production reservation policy;
- connection safety uses the normal production cap (128 total established by default), not an example-specific low-CPU ceiling;
- CPU optimizations coalesce observability and peer-cache persistence, suppress duplicate Identify dashboard work, and avoid connection-triggered DHT refresh churn instead of reducing protocol capability;
- native WebRTC-direct bounds/ages unverified half-open UDP state and explicitly cleans failed, cancelled, and dropped peer connections instead of retaining transport resources across churn;
- manual `bootstrap_peers` are empty;
- manual `relay_peers` are empty;
- public fallback mode is `fallback_only`;
- public auto-connect is enabled;
- rendezvous client behavior is enabled;
- rendezvous server behavior is disabled;
- DHT provider discovery is enabled;
- discovery namespace tags stay hashed;
- public rendezvous and relay slots remain available for real app-operated or contracted infrastructure.

Power users can still override the defaults by adding manual peers or disabling public fallback.

## How default connectivity progresses beyond bootstrap

Public bootstrap peers are only the entry point into the wider routing layer. The default runtime continues automatically through the remaining stages rather than requiring the user to configure rendezvous or relay peers:

| Piece | Purpose |
|---|---|
| Same-LAN UDP discovery | Find compatible peers on the local network immediately; the official Android Emulator uses the same protocol through a `10.0.2.2` unicast reply assist. |
| Public bootstrap | Reach the wider libp2p/Kademlia layer when LAN discovery is not enough. |
| Network-scoped DHT provider discovery | Find only application peers for the exact `network_id`/app namespace. |
| Identity-signed DHT address records | Recover direct and relay dial routes without trusting an unauthenticated address publisher. |
| Dynamic Circuit Relay v2 discovery | Reserve public relay-hop-capable DHT peers when direct reachability is unavailable. |
| DCUtR | Try to upgrade a relayed connection into a direct connection while retaining relay fallback. |
| Auto-connect policy | Turn discovered app peers into bounded network-layer dial attempts. |
| Optional rendezvous/owned relay infrastructure | Add deterministic service capacity when an operator wants it; not required for the built-in discovery path. |

The shared repository ships public bootstrap defaults but does not pretend to operate a public rendezvous or relay fleet. Fresh nodes progress from bootstrap into network-scoped DHT provider/address discovery, dynamic public relay-hop discovery, and DCUtR fallback. Same-LAN peers use the faster UDP path. Applications may still add owned rendezvous or relay/mediator DNSADDR entries when they want deterministic infrastructure capacity or policy control.

## Advanced overrides

Advanced users and operators may disable or replace the consumer default:

```json
{
  "bootstrap_peers": [
    "/dnsaddr/bootstrap.example.net/p2p/<peer-id>"
  ],
  "discovery": {
    "rendezvous_peers": [
      "/dnsaddr/rendezvous.example.net/p2p/<peer-id>"
    ],
    "public_bootstrap": {
      "mode": "disabled",
      "bootstrap_seed_peers": [],
      "rendezvous_peers": [],
      "relay_peers": [],
      "auto_connect_discovered_peers": false
    }
  },
  "relay_peers": [
    "/dnsaddr/relay.example.net/p2p/<peer-id>"
  ]
}
```

That shape is the private/operator path. See `PRIVATE_INFRASTRUCTURE_FIRST.md` and `examples/private-infrastructure-first.config.json` for a complete advanced example.

### Public DHT meetup retries

Consumer mode does not assume one startup DHT query is enough. After the initial announce/query, startup retries back off through 5, 15, 30, and 60 seconds before settling on `discovery.dht.refresh_interval_secs`. Recovery from zero connected peers can pull the next refresh forward, subject to a 5-second minimum gap, and learning public external addresses triggers an immediate refresh. Ordinary additional connections do not restart the DHT refresh timer. This lets nodes meet as routing improves without tying DHT work to every application heartbeat.

When no separate public relay fleet is configured, Identify responses from public-DHT infrastructure are inspected for Circuit Relay v2 hop support. Compatible candidates are bounded and reservation attempts fail visibly while DHT discovery and direct-dial attempts continue. They are infrastructure only, never trusted application contacts.
