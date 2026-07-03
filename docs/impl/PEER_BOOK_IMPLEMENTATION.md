# Peer book implementation

The peer book lives in `crates/connectivity/peer_book.rs`.

Runtime sources update it through the node event loop:

- startup bootstrap/rendezvous/cache/relay candidates are recorded before the event loop starts
- connection events mark peers connected or disconnected
- identify events record advertised listen addresses
- rendezvous discovery records peers, addresses, and namespaces
- DHT provider-record discovery records provider peers and namespaces
- command handling reads the peer book for `get_peers()`
- connection planning reads peer-book addresses and relay preference hints when `connect_peer(...)` is called

The peer book is currently in-memory. Durable peer address storage remains handled by the existing peer-cache module. This keeps `get_peers()` fast and avoids expanding the on-disk cache format into an application-visible database.

Snapshot fields:

- `peer_book_known_peers`
- `peer_book_discovered_peers`

Prometheus metrics:

- `p2p_peer_book_known_peers`
- `p2p_peer_book_discovered_peers`

## Peer-cache implementation notes

`crates/connectivity/peer_cache.rs` uses a v2 cache shape with separate `identities` and `dialable_addrs` sections. The older-format `entries` field is still written/read for backward-compatible diagnostics, but v2 code deduplicates it with `dialable_addrs` before applying freshness policy.

Dialable address retention is kind-specific:

- `public_direct`: capped by `discovery.peer_cache_public_addr_max_age_secs`;
- `relay_reservation`: capped by `discovery.peer_cache_relay_addr_max_age_secs` or an explicit expiry supplied by the caller;
- `local_session`: rejected on load unless `discovery.peer_cache_persist_local_addrs` is true.

This prevents stale NAT/public-IP announcements, expired relay reservations, and same-machine loopback/LAN addrs from being reused as if they were long-lived app-peer routes.
