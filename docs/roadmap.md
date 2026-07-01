# Roadmap: resilient public-bootstrap discovery fallback

This roadmap tracks the next discovery layer: prefer our own bootstrap, rendezvous, mediator, and relay infrastructure first, then use explicitly enabled public libp2p/IPFS bootstrap and relay resources only as a fallback when our own infrastructure and local peer cache are unavailable.

## Target behavior

1. A node first dials operator-pinned bootstrap, rendezvous, mediator, and relay peers.
2. If those are unavailable, it uses cached healthy peers.
3. If the mesh is still empty and public fallback is enabled, it dials public bootstrap seeds.
4. Nodes publish hashed application rendezvous/DHT namespace keys instead of raw human-readable contact tags.
5. `get_peers()` returns connected, cached, rendezvous-discovered, DHT-provider-discovered, relay-discovered, and manually configured peers with source metadata.
6. `connect_peer(...)` uses a connection plan: direct addresses first, relayed addresses next, DCUtR upgrade after a relay path exists, and relay fallback retained when DCUtR fails.

## Phase 1 — Peer roles and API metadata

Status: complete.

- Add explicit peer-source metadata shared by the public API and future peer book.
- Extend `PeerInfo` so `get_peers()` can represent discovered peers, not only connected peers.
- Keep the six primitives unchanged.
- Document the difference between bootstrap, rendezvous, relay/mediator, DHT-provider, cached, manual, and connected peers.
- Add QA coverage so this metadata cannot regress.

## Phase 2 — Hashed discovery namespace model

Status: complete.

- Add a deterministic namespace builder: `p2p-net/<network_id>/<app_id>/<hashed_tag>`.
- Add config for app discovery tags without publishing raw contact names or invite phrases.
- Support multiple tags per application.
- Keep readable namespaces available only behind an explicit unsafe/debug option.
- Runtime rendezvous refresh now uses derived hashed namespaces when app tags are configured, otherwise it uses the operator rendezvous namespace.

## Phase 3 — Public fallback policy

Status: complete.

- Added `discovery.public_bootstrap` with modes: `disabled`, `fallback_only`, and `always`.
- Operator-owned bootstrap, rendezvous, relay, and cached peers are preferred before public fallback.
- Public bootstrap and public relay/mediator candidates are explicit config lists, not hidden defaults.
- Snapshots and metrics report whether public fallback was used and which public candidate class participated.

## Phase 4 — DHT provider-record namespace discovery

Status: planned.

- Announce hashed namespace keys through Kademlia provider records.
- Query providers for hashed namespace keys when rendezvous peers are unavailable or empty.
- Populate the peer book with DHT-provider results.
- Keep DHT discovery internal; the public API remains the six primitives.

## Phase 5 — Peer book

Status: planned.

- Add a single internal peer book for connected, cached, rendezvous, DHT provider, relay, bootstrap, and manual peer records.
- Track addresses, sources, namespaces, capability hints, last-seen timestamps, connection state, and failures.
- Make `get_peers()` read from the peer book instead of only the live swarm connection set.

## Phase 6 — Connection strategy planner

Status: planned.

- Add direct/relay/DCUtR connection planning behind `connect_peer(...)`.
- Prefer QUIC/direct addresses when available.
- Use relay paths when direct dialing fails or the peer is known to be behind NAT/CGNAT.
- Attempt DCUtR after a relay path exists.
- Keep relay fallback when DCUtR fails.

## Phase 7 — Network resurrection tests

Status: planned.

- Add QA tests for a Joe/Alice namespace-discovery flow.
- Verify public fallback is used only when policy allows it.
- Verify raw tags are not published by default.
- Verify discovered-but-not-connected peers appear in `get_peers()`.
- Verify `connect_peer(...)` can consume peer-book records through the connection planner.

## Phase 8 — Operator docs and examples

Status: planned.

- Add examples for private-infrastructure-first mode.
- Add examples for public-fallback mode.
- Document privacy, abuse, performance, and dependency tradeoffs of public bootstrap/relay fallback.
- Document production guidance for running our own bootstrap/rendezvous/mediator fleet.
