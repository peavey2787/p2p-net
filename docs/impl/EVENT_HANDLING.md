# Swarm event handling layout

The node runtime splits swarm event handling by responsibility.
The top-level dispatcher remains `crates/node/events.rs`; it should only route
`SwarmEvent<MeshEvent>` values to focused handlers.

## Modules

- `crates/node/events/connection.rs` handles connection lifecycle, AutoNAT snapshot
  updates, local/public listen-address classification, relayed listen addresses,
  connection caps, and refresh-on-listen rendezvous startup actions.
- `crates/node/events/connection/errors.rs` handles incoming and outgoing connection
  failures, fallback dialing, and DHT provider auto-connect retry scheduling.
- `crates/node/events/relay_client.rs` handles relay-client reservation and circuit
  events.
- `crates/node/events/relay_server.rs` handles relay-server/mediator events,
  denial health mapping, and schedule enforcement.
- `crates/node/events/dcutr.rs` handles DCUtR attempt/success accounting.
- `crates/node/events/rendezvous.rs` handles rendezvous client/server event snapshot
  synchronization.
- `crates/node/events/kademlia.rs` handles DHT provider-record announcements,
  provider lookups, routing updates, and provider-discovery metrics.
- `crates/node/events/gossip.rs` handles heartbeat gossip validation against the signed
  gossipsub author, replay decisions, reputation changes, and validation reporting
  against the immediate propagation peer.
- `crates/node/events/app.rs` binds application envelopes to the signed author and
  outer topic, enforces timestamp/nonce replay protection, reports explicit manual
  gossipsub validation decisions, and delivers accepted addressed/broadcast messages
  to local subscribers when applicable.

## Design rules

- Do not add protocol-specific logic back into `crates/node/events.rs`.
- Do not make relay handlers update rendezvous state or gossip handlers update
  relay state.
- Keep `SwarmEventContext` as the shared state boundary unless runtime services
  are intentionally moved into narrower structs.
- New libp2p behaviours should get their own focused event module unless their
  handling is truly only a one-line dispatch to `stack`.
- New application protocols should build above the public data-plane primitives instead of
  adding topic-specific logic to the dispatcher.

This split is intentionally behavior-preserving. Resolved capability-driven behaviour construction can evolve without editing one large god file.
