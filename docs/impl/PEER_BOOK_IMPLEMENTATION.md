# Peer book implementation

The peer book lives in `crates/connectivity/peer_book.rs`.

Runtime sources update it through the node event loop:

- startup bootstrap/rendezvous/cache/relay candidates are recorded before the event loop starts
- connection events mark peers connected or disconnected
- identify events record advertised listen addresses
- rendezvous discovery records peers, addresses, and namespaces
- DHT provider-record discovery records provider peers and namespaces
- command handling reads the peer book for `get_peers()`

The peer book is currently in-memory. Durable peer address storage remains handled by the existing peer-cache module. This keeps `get_peers()` fast and avoids expanding the on-disk cache format into an application-visible database.

Snapshot fields:

- `peer_book_known_peers`
- `peer_book_discovered_peers`

Prometheus metrics:

- `p2p_peer_book_known_peers`
- `p2p_peer_book_discovered_peers`
