# General-purpose application API

`p2p-net` exposes one stable, application-facing API surface. Applications should build on these six data-plane primitives instead of depending on libp2p swarm internals:

1. `connect_peer`
2. `disconnect_peer`
3. `send_message`
4. `broadcast`
5. `subscribe`
6. `get_peers`

These primitives are available on `NodeHandle`, returned by `start_node(...)` or `start_node_with_platform(...)`. A seventh query/management primitive, `get_metrics(peer_id)`, exposes runtime-owned infrastructure telemetry without pushing payment or settlement logic into the networking core.

## Primitive semantics

### `connect_peer(addr)`

Dials a concrete peer multiaddr. Production callers should prefer addresses that include `/p2p/<PeerId>` so the transport can authenticate the expected peer identity. When the target peer id is known, the node may expand this request through the internal connection planner: direct QUIC addresses are preferred, other direct addresses follow, and relayed paths are retained as fallback candidates. Relay-preferred peers can use relayed paths first, and relayed connections feed the DCUtR upgrade policy after the relay path exists.

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

The current transport carrier is signed/authenticated gossipsub. Receivers bind the envelope's `source_peer_id` to the cryptographically authenticated gossipsub author and bind the envelope topic to the outer gossipsub topic before accepting it. Signed application envelopes are also freshness-checked and passed through a bounded nonce/timestamp replay cache. Addressed messages are delivered locally only when the target peer id matches the local node, while valid messages for other peers remain eligible for mesh forwarding.

**Security boundary:** `send_message` is addressed delivery, not confidential point-to-point transport. Other subscribed gossipsub peers may receive/carry the payload while forwarding it. Applications that require confidentiality must encrypt the payload end-to-end or use/build a direct encrypted stream/request-response protocol.

### `broadcast(topic, payload)`

Publishes an untargeted `AppMessage` envelope to all peers subscribed to the same application topic.

### `subscribe(topic)`

Subscribes the local swarm to an application topic and returns a topic-filtered local `AppSubscription` for incoming `AppMessage` envelopes delivered to this process.

### `get_peers()`

Returns known peers as `PeerInfo` records. This includes connected, cached, rendezvous-discovered, DHT-provider-discovered, relay-discovered, bootstrap, bootstrap-seed, and configured peers when those sources are available. `PeerInfo.sources` tells applications how each peer was learned.

### `get_metrics(peer_id)`

Returns `NodeMetrics` from the node runtime. Passing `None` returns global totals plus per-peer and per-topic bandwidth maps. Passing `Some(peer_id)` returns the same top-level counters but filters the per-peer map to that peer and omits per-topic details to avoid large result payloads.

Metrics currently include:

- bandwidth totals for accounted ingress/egress bytes, plus per-peer and per-topic maps where available;
- storage counters for node-managed persisted chunks/write payloads;
- compute counters for event-loop work estimates, active request pressure, and connection-cap choking.

Applications can use these counters to build quota, billing, or settlement systems above the networking core. Wallets, tokens, and transaction logic are intentionally outside this crate.

## Peer metadata

`PeerInfo` contains the peer id, connection status, known addresses, optional capability hints, optional namespace metadata, and one or more `PeerSource` values. This keeps `get_peers()` stable while the internal peer book merges multiple discovery sources into one application-facing record per peer id.

`PeerSource` values are:

- `connected`
- `bootstrap`
- `bootstrap_seed`
- `public_bootstrap_seed`
- `rendezvous`
- `public_rendezvous`
- `dht_provider`
- `relay_discovery`
- `public_relay_discovery`
- `peer_cache`
- `manual`

## Topic rules

Application topics are normalized and namespaced as:

```text
p2p-net/app/v2/net-<network_id>/<topic>
```

Topics must be non-empty, no more than 128 bytes, and contain only ASCII letters, numbers, `-`, `_`, `.`, or `/`.

## Payload rules

Application payloads are raw bytes and are capped at 1 MiB by default. Higher-level applications own their own payload schema, encryption, compression, chunking, and authorization policy.

## Design boundary

The six data-plane primitives are intentionally small. Chat, games, decentralized storage, compute, pub/sub, databases, and mesh applications should implement their domain protocol above this layer. Telemetry remains query-only so application billing/settlement policy can evolve independently from transport code.
