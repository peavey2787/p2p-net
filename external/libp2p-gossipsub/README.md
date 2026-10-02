# p2p-net-gossipsub

`p2p-net-gossipsub` is the Gossipsub companion for `p2p-net`. It is rust-libp2p
`libp2p-gossipsub` 0.50.0 (MIT) with a small, audited patch, and it keeps the
public Rust library name `libp2p_gossipsub` so it stays a drop-in for the
rust-libp2p 0.57 generation.

## What the patch fixes

**Messages stranded on a peer's remaining connection.** Gossipsub sends its
subscriptions only on the first connection to a peer, so a later connection's
handler has no outbound stream. When the first connection closes — a relayed
connection replaced by a direct one (DCUtR, or the browser `/webrtc` upgrade) —
every message for that peer is queued for the remaining handler. Upstream only
checked `is_empty()` there, without registering a waker, so the messages waited
until some unrelated event happened to poll the connection, and publishes
expired (`publish_queue_duration`) before that. The handler now uses
`Queue::poll_is_empty`, which registers its waker while the queue is empty, and
`Shared::poll_pop` replaces a stale registered waker instead of keeping it.

The change is confined to `src/queue.rs` and `src/handler.rs` (marked
`p2p-net:`). `tests/connection_migration.rs` reproduces the stall
deterministically: it fails on upstream 0.50.0 and passes here. The upstream
unit and smoke tests are unchanged and pass.

Application developers normally do **not** depend on this crate directly. Add
`p2p-net = "0.1.0"` to the application's `Cargo.toml`; Cargo resolves this
companion automatically.
