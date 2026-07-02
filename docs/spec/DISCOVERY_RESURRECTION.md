# Discovery resurrection model

The discovery layer is designed to keep the six public primitives stable while allowing the node to recover a mesh after operator-owned infrastructure is temporarily unavailable.

## Infrastructure preference order

Nodes should prefer infrastructure in this order:

1. Operator-owned bootstrap peers.
2. Operator-owned rendezvous peers.
3. Operator-owned mediators/relays.
4. Healthy local peer-cache records.
5. Public fallback bootstrap/relay candidates when normal app policy allows them.

Public fallback is the normal consumer discovery entry point, but it is not a replacement for operating reliable infrastructure when an application needs guaranteed service capacity or stronger metadata control.

## Peer roles

| Role | Purpose |
|---|---|
| Connected peer | A peer with an active swarm connection. |
| Bootstrap peer | A trusted configured peer used to enter the network. |
| Bootstrap seed peer | A seed peer used to jump-start Kademlia/routing-table discovery. |
| Rendezvous peer | A peer that accepts namespace registration and discovery requests. |
| DHT provider peer | A peer discovered as a provider of a hashed application namespace key. |
| Relay-discovery peer | A peer learned as a potential relay/mediator candidate. |
| Cached peer | A previously seen identity-bound address loaded from local storage. |
| Manual peer | A peer supplied directly by the user or embedding application. |

These roles are exposed through `PeerSource` metadata on `PeerInfo`. A single peer may eventually have multiple sources.

## Rendezvous tags and privacy

Applications should not publish raw contact names, invite phrases, or human-readable rendezvous tags by default. The target model is to derive namespace keys from:

```text
network_id + app_id + secret/contact tag
```

and publish only hashed namespace keys. The implemented namespace shape is `p2p-net/<network_id>/<app_id>/<tag_fingerprint>`; see `docs/spec/DISCOVERY_NAMESPACES.md`.

## CGNAT-to-CGNAT reality

Two nodes that are both behind CGNAT usually still need a reachable third-party path before DCUtR can try to upgrade the connection. That path can be one of our own mediators/relays, or a public relay fallback when public relay candidates are configured. Once a relayed connection exists, DCUtR can attempt a direct upgrade. If the upgrade fails, the relay path remains the fallback.

## Public fallback policy

The implemented public fallback policy lives under `discovery.public_bootstrap`. It defaults to `fallback_only` with built-in public bootstrap seeds for normal app mode. Private-infrastructure-first operators set `mode = "disabled"` and configure owned bootstrap, rendezvous, mediator, and relay peers. See `docs/spec/PUBLIC_FALLBACK.md`.

## DHT provider fallback

DHT provider discovery lets a node announce and query hashed application namespace keys through Kademlia provider records. This is not a public API surface; it is fallback discovery plumbing behind the six primitives.

Startup uses DHT provider records after deriving the same hashed namespaces used by rendezvous discovery. Provider lookups run when rendezvous peers are absent by default, or alongside rendezvous when explicitly configured. See `docs/spec/DHT_PROVIDER_DISCOVERY.md`.

## Resilience validation

The `network_resurrection` QA suite covers the recovery path that matters to applications: two users deriving the same hashed contact namespace, rediscovering a not-yet-connected peer from rendezvous/DHT-style metadata, exposing that peer through the peer-book view used by `get_peers()`, and feeding stored addresses into the `connect_peer(...)` planner.

The same suite also checks that public bootstrap/relay fallback participates only when policy allows it and owned/cached candidates do not already satisfy fallback-only mode.
