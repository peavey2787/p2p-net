# Connection strategy implementation

The planner lives in `crates/connectivity/connection_strategy.rs` and is used by node command routing for `connect_peer(...)`.

## Runtime flow

1. `NodeHandle::connect_peer(addr)` sends `NodeCommand::ConnectPeer` unchanged.
2. Command handling records the requested address as a manual peer-book source when it includes a target peer id.
3. `build_connection_plan(...)` merges the requested address with peer-book addresses for that target peer.
4. The plan orders candidates by connection type: direct QUIC, direct, relay, unless the record is relay-preferred.
5. The command handler queues the first dialable attempt with `Swarm::dial` and stores remaining attempts in `PendingConnectionPlans`.
6. `OutgoingConnectionError` advances the pending plan to the next candidate for that peer.
7. `ConnectionEstablished` clears the pending plan for the peer.
8. Existing relayed-connection handling applies the DCUtR policy and relay fallback counters.

## Peer-book support

`PeerRecord` now includes an internal `relay_preferred` hint. This is intentionally not exposed as a public `PeerInfo` field; applications already see the address list and source metadata, while the internal planner can use the hint to prefer relay paths for restrictive-NAT peers.

Capability hints remain exposed through `PeerInfo`:

- `supports_relay`
- `supports_rendezvous`
- `supports_dcutr`

## Validation coverage

`qa/tests/connection_strategy.rs` covers:

- direct QUIC before direct TCP before relay
- relay-preferred peers using relay before direct candidates
- DCUtR planner flags following operator policy

The module also has unit coverage for pending-plan fallback advancement.
