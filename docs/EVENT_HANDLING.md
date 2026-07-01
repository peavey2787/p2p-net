# Swarm event handling layout

The node runtime splits swarm event handling by responsibility.
The top-level dispatcher remains `src/node/events.rs`; it should only route
`SwarmEvent<MeshEvent>` values to focused handlers.

## Modules

- `src/node/events/connection.rs` handles connection lifecycle, listener errors,
  AutoNAT snapshot updates, relayed listen addresses, connection caps, and
  refresh-on-listen rendezvous startup actions.
- `src/node/events/relay_client.rs` handles relay-client reservation and circuit
  events.
- `src/node/events/relay_server.rs` handles relay-server/mediator events,
  denial health mapping, and schedule enforcement.
- `src/node/events/dcutr.rs` handles DCUtR attempt/success accounting.
- `src/node/events/rendezvous.rs` handles rendezvous client/server event snapshot
  synchronization.
- `src/node/events/gossip.rs` handles heartbeat gossip validation, replay
  decisions, reputation changes, and explicit gossipsub validation reporting.

## Design rules

- Do not add protocol-specific logic back into `src/node/events.rs`.
- Do not make relay handlers update rendezvous state or gossip handlers update
  relay state.
- Keep `SwarmEventContext` as the shared state boundary unless runtime services
  are intentionally moved into narrower structs.
- New libp2p behaviours should get their own focused event module unless their
  handling is truly only a one-line dispatch to `stack`.

This split is intentionally behavior-preserving. Resolved capability-driven behaviour construction can evolve without editing one large god file.
