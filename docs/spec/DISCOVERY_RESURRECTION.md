# Discovery resurrection model

The discovery layer is designed to keep the public application API stable while allowing the node to recover a mesh after operator-owned infrastructure is temporarily unavailable.

## Infrastructure preference order

Nodes should prefer infrastructure in this order:

1. Operator-owned bootstrap peers.
2. Operator-owned rendezvous peers.
3. Operator-owned mediators/relays.
4. Healthy local peer-cache records.
5. Public bootstrap entry plus network-scoped DHT provider/address discovery and dynamically discovered relay-hop peers when normal app policy allows them.

Public fallback is the normal consumer discovery entry point, but it is not a replacement for operating reliable infrastructure when an application needs guaranteed service capacity or stronger metadata control.

## Peer roles

| Role | Purpose |
|---|---|
| Connected peer | A peer with an active swarm connection. |
| Bootstrap peer | A trusted configured peer used to enter the network. |
| Bootstrap seed peer | A seed peer used to jump-start Kademlia/routing-table discovery. |
| Public bootstrap seed peer | A seed peer supplied by the public fallback policy rather than private/operator config. |
| Rendezvous peer | A peer that accepts namespace registration and discovery requests. |
| Public rendezvous peer | A rendezvous peer supplied by the public fallback policy rather than private/operator config. |
| DHT provider peer | A peer discovered as a provider of a hashed application namespace key. |
| Relay-discovery peer | A peer learned as a potential relay/mediator candidate. |
| Public relay-discovery peer | A relay/mediator candidate supplied by public fallback rather than private/operator config. |
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

Two nodes that are both behind CGNAT still need a reachable third-party path before DCUtR can try to upgrade the connection. That path can be an operator relay/mediator or a Circuit Relay v2 hop peer discovered dynamically through the public DHT. Once a relayed connection exists, DCUtR can attempt a direct upgrade. If the upgrade fails, the relay path remains the fallback.

## Public fallback policy

The implemented public fallback policy lives under `discovery.public_bootstrap`. It defaults to `fallback_only` with built-in public bootstrap seeds. Fresh default nodes then use network-scoped DHT provider/address discovery and can reserve relay-hop-capable peers learned dynamically from the public DHT; the shared repository does not require a built-in project-operated rendezvous or relay list. Private-infrastructure-first operators set `mode = "disabled"` and configure owned bootstrap, rendezvous, mediator, and relay peers. See `docs/spec/PUBLIC_FALLBACK.md`.

## DHT provider fallback

DHT provider discovery lets a node announce and query hashed application namespace keys through Kademlia provider records. This is not a public API surface; it is fallback discovery plumbing behind the data-plane primitives.

Startup uses DHT provider records after deriving the same hashed namespaces used by rendezvous discovery. Provider lookups run when rendezvous peers are absent by default, or alongside rendezvous when explicitly configured. See `docs/spec/DHT_PROVIDER_DISCOVERY.md`.

## Resilience validation

The `network_resurrection` QA suite covers metadata-level recovery, while `swarm_smoke` and `live_two_node_probe` cover the production auto-connect path. The zero-manual-dial probes use isolated identities and require application-peer connectivity without calling `connect_peer(...)`.

The same suite also checks that public bootstrap/rendezvous/relay fallback participates only when policy allows it and owned/cached candidates do not already satisfy fallback-only mode.
