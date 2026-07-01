# Peer book specification

The peer book is the single internal index used by the six-primitives API to describe peers known to the node.

## Purpose

`get_peers()` must not be limited to live swarm connections. It returns peers learned from any trusted discovery source:

- connected peers
- manually configured bootstrap peers
- bootstrap seed peers
- configured rendezvous peers
- rendezvous-discovered peers
- DHT provider-record peers
- relay-discovery peers
- healthy peer-cache entries

## Record model

Each record tracks:

- peer id
- known multiaddrs
- discovery sources
- discovery namespaces
- connection state
- optional capability hints for relay, rendezvous, and DCUtR
- internal relay-preferred hint for peers known to need relay-first dialing
- last-seen time
- failure count

When multiple sources report the same peer id, the peer book merges them into one record instead of duplicating application-facing peer entries.

## API behavior

The public API remains:

```rust
let peers = node.get_peers().await?;
```

The returned `PeerInfo` values include source metadata so application code can choose whether to connect only to direct peers, namespace-matched peers, relay-capable peers, or all known peers.

## Privacy

The peer book stores already-derived namespace identifiers. Raw contact tags or invite phrases are not stored by the peer book when the discovery namespace layer is configured in hashed mode.
