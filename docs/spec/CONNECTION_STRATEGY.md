# Connection strategy planner specification

`connect_peer(...)` remains one of the six public application data-plane primitives. The public call accepts a concrete multiaddr, while the node event loop may expand that request into an ordered internal dial plan using the peer book.

## Goals

The planner provides deterministic connection ordering without exposing libp2p internals to app shells:

- prefer direct QUIC addresses when they are known
- try other direct addresses before relay paths for normal peers
- prefer relay paths when the peer book marks a peer as relay-preferred
- keep relayed addresses in the plan so they can be used when direct dialing fails
- mark relayed connections as eligible for DCUtR policy after a relay path exists
- retain relay fallback when DCUtR is disabled, unavailable, or fails

## Inputs

A plan is built from:

- the requested `connect_peer(...)` multiaddr
- known peer-book addresses for the same target peer id
- the internal relay preference hint on the peer-book record
- the operator DCUtR policy

The requested address is always included. Peer-book addresses are only merged when the requested address contains a `/p2p/<PeerId>` component so the planner can safely identify the target record.

## Attempt ordering

For normal peers, attempts are ordered as:

1. direct QUIC addresses
2. other direct addresses
3. relayed `/p2p-circuit` addresses

For relay-preferred peers, relayed `/p2p-circuit` addresses are tried first, followed by direct QUIC and then other direct addresses. This covers peers that are known or strongly suspected to be behind restrictive NAT/CGNAT.

Duplicate addresses are removed before ordering.

## Failure behavior

The event loop dials the first viable attempt. If that asynchronous outgoing connection fails and the target peer id is known, the pending plan advances to the next candidate. This gives `connect_peer(...)` direct-first behavior while retaining relay fallback when direct dialing fails.

Immediate dial-construction errors are handled synchronously by trying the next candidate in the same plan. The public command fails only when no candidate can even be queued for dialing.

## DCUtR behavior

The planner does not run the hole-punch protocol directly. It records whether the operator policy allows DCUtR after a relay path exists. When a relayed connection is established, the existing connection event policy treats that connection as DCUtR-eligible, applies the per-peer retry budget, and keeps the relay circuit as fallback when configured.
