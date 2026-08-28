# External / companion source

Audited third-party-derived source and publishable companion crates live here.
The hardened `p2p-net-webrtc` companion is published independently because its
source is derived from the MIT-licensed rust-libp2p WebRTC transport, while the
root `p2p-net` crate keeps its own package/license boundary. The
`libp2p-dns` and `libp2p-mdns-placeholder` directories are non-publishable,
lock-resolution-only patches used by the checked-in `.cargo/config.toml` so
rust-libp2p 0.56 weak optional feature references cannot reintroduce Hickory
into the repository lockfile. They are excluded from the `p2p-net` crates.io
payload and are not required by downstream applications. Application runtime
code that is not such a companion should remain under `crates/`.
