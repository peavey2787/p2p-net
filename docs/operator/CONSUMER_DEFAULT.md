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

## Why public bootstrap alone is not enough

Public bootstrap peers help a node enter the wider routing layer, but they do not guarantee that two fresh app installs behind different NATs can connect to each other.

Reliable no-config consumer connectivity needs more than bootstrap:

| Piece | Purpose |
|---|---|
| Public bootstrap | Reach the wider libp2p/Kademlia layer. |
| Public app rendezvous | Let fresh app nodes register and find peers in the app namespace. |
| DHT provider discovery | Recover app peers through hashed namespace provider records. |
| Public relay/mediator | Provide a reachable third-party path when both users are behind NAT/CGNAT. |
| DCUtR | Try to upgrade a relayed connection into a direct connection. |
| Auto-connect policy | Turn discovered app peers into bounded network-layer dial attempts. |

The shared repository ships public bootstrap defaults and the config slots for public rendezvous and relay candidates. It does not pretend to operate a public rendezvous or relay fleet. Applications that require reliable run-two-fresh-installs behavior should add real public rendezvous and relay/mediator DNSADDR entries before release.

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
