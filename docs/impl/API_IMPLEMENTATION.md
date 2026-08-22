# Application API implementation

The general-purpose API is implemented without exposing libp2p internals to embedders.

## Public surface

`NodeHandle` exposes:

```rust
connect_peer(addr)
disconnect_peer(peer_id)
send_message(peer_id, topic, payload)
broadcast(topic, payload)
subscribe(topic)
get_peers()
get_metrics(peer_id)
```

## Internal flow

1. Public methods send a `NodeCommand` through the node command channel.
2. The swarm task processes commands on the same Tokio task that owns `Swarm<MeshBehaviour>`.
3. App messages are encoded as `AppMessage` envelopes and published through app-topic gossipsub topics.
4. Subscribed app topics are tracked by topic hash, separate from heartbeat gossip.
5. Incoming app-topic messages are decoded by `node/events/app.rs`; the claimed source and topic are bound to the signed gossipsub author/outer topic, freshness and bounded replay checks are enforced, every message receives an explicit manual-validation decision, and local delivery occurs only when network/target policy matches.
6. `get_peers()` reads the internal peer book, which merges connected, cached, configured, rendezvous, DHT-provider, and relay-discovery records.
7. `get_metrics(peer_id)` returns a clone of the runtime-owned `NodeMetrics`; `Some(peer_id)` filters the per-peer bandwidth map for cheaper queries.
8. `NodeSnapshot` exposes API/app/operator counters for observability.

This preserves SRP:

- `crates/api/` owns public envelope/topic types.
- `crates/api/metrics.rs` owns the public telemetry data structures.
- `crates/node/handle.rs` owns public primitive methods and command types.
- `crates/node/commands.rs` owns command execution against the swarm.
- `crates/node/events/app.rs` owns incoming app-message delivery.
- `crates/stack/` remains transport/behaviour construction only.
