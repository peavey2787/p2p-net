# Specifications

Core protocol and lifecycle specifications. Documents here define behavior that
implementation and validation work must preserve.

- `API_PRIMITIVES.md` — the six public application primitives and app message envelope.
- `DISCOVERY_NAMESPACES.md` — hashed app/contact/group discovery namespace model.
- `DISCOVERY_RESURRECTION.md` — private-infrastructure-first discovery fallback, peer roles, and CGNAT/DCUtR reality model.
- `DHT_PROVIDER_DISCOVERY.md` — Kademlia provider-record discovery for hashed app namespaces.
- `PEER_BOOK.md` — normalized peer metadata returned by `get_peers()`.
- `CONNECTION_STRATEGY.md` — direct/relay/DCUtR planning behind `connect_peer(...)`.
- `IDENTITY_KEY_BACKUP_ROTATION.md` — identity key backup and rotation rules.
- `PUBLIC_FALLBACK.md` documents explicit public bootstrap and relay fallback.
