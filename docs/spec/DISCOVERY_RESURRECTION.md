# Discovery resurrection model

The discovery layer is designed to keep the six public primitives stable while allowing the node to recover a mesh after operator-owned infrastructure is temporarily unavailable.

## Infrastructure preference order

Nodes should prefer infrastructure in this order:

1. Operator-owned bootstrap peers.
2. Operator-owned rendezvous peers.
3. Operator-owned mediators/relays.
4. Healthy local peer-cache records.
5. Explicitly enabled public libp2p/IPFS bootstrap or relay fallback.

Public fallback is not a replacement for operating reliable infrastructure. It is a recovery path for small networks, demos, disaster recovery, and last-resort discovery.

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

Two nodes that are both behind CGNAT usually still need a reachable third-party path before DCUtR can try to upgrade the connection. That path can be one of our own mediators/relays, or an explicitly enabled public relay fallback. Once a relayed connection exists, DCUtR can attempt a direct upgrade. If the upgrade fails, the relay path remains the fallback.
