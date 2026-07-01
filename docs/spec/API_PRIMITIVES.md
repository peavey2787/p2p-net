# General-purpose application API

`p2p-net` exposes one stable, application-facing API surface. Applications should build on these six primitives instead of depending on libp2p swarm internals:

1. `connect_peer`
2. `disconnect_peer`
3. `send_message`
4. `broadcast`
5. `subscribe`
6. `get_peers`

These primitives are available on `NodeHandle`, returned by `start_node(...)` or `start_node_with_platform(...)`.

## Primitive semantics

### `connect_peer(addr)`

Dials a concrete peer multiaddr. Production callers should prefer addresses that include `/p2p/<PeerId>` so the transport can authenticate the expected peer identity.

### `disconnect_peer(peer_id)`

Closes active swarm connections to a peer id.

### `send_message(peer_id, topic, payload)`

Publishes an addressed `AppMessage` envelope on an application topic. The envelope includes:

- schema version
- network id
- topic
- source peer id
- optional target peer id
- timestamp
- nonce
- raw payload bytes

The current transport carrier is authenticated gossipsub. Receivers deliver addressed messages only when the target peer id matches the local node. This gives applications a stable unicast-style primitive while keeping the underlying transport replaceable.

### `broadcast(topic, payload)`

Publishes an untargeted `AppMessage` envelope to all peers subscribed to the same application topic.

### `subscribe(topic)`

Subscribes the local swarm to an application topic and returns a topic-filtered local `AppSubscription` for incoming `AppMessage` envelopes delivered to this process.

### `get_peers()`

Returns known peers as `PeerInfo` records. This includes connected, cached, rendezvous-discovered, DHT-provider-discovered, relay-discovered, bootstrap, bootstrap-seed, and configured peers when those sources are available. `PeerInfo.sources` tells applications how each peer was learned.

## Peer metadata

`PeerInfo` contains the peer id, connection status, known addresses, optional capability hints, optional namespace metadata, and one or more `PeerSource` values. This keeps `get_peers()` stable while the internal peer book merges multiple discovery sources into one application-facing record per peer id.

`PeerSource` values are:

- `connected`
- `bootstrap`
- `bootstrap_seed`
- `rendezvous`
- `dht_provider`
- `relay_discovery`
- `peer_cache`
- `manual`

## Topic rules

Application topics are normalized and namespaced as:

```text
p2p-net/app/v1/net-<network_id>/<topic>
```

Topics must be non-empty, no more than 128 bytes, and contain only ASCII letters, numbers, `-`, `_`, `.`, or `/`.

## Payload rules

Application payloads are raw bytes and are capped at 1 MiB by default. Higher-level applications own their own payload schema, encryption, compression, chunking, and authorization policy.

## Design boundary

The six primitives are intentionally small. Chat, games, decentralized storage, compute, pub/sub, databases, and mesh applications should implement their domain protocol above this layer.
