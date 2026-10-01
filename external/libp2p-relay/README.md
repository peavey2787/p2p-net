# p2p-net-relay

`p2p-net-relay` is the Circuit Relay v2 companion for `p2p-net`. It is
rust-libp2p `libp2p-relay` 0.22.0 (MIT) with a small, audited patch, and it
keeps the public Rust library name `libp2p_relay` so it stays a drop-in for the
rust-libp2p 0.57 generation.

## What the patch adds

1. **Exact per-circuit relay metering.** Every accepted circuit gets a
   `CircuitId` and a live `Arc<CircuitUsage>` with separate `src_to_dst` and
   `dst_to_src` counters, written by the relay data path at the moment it
   forwards bytes. `Event::CircuitReqAccepted` carries the id and the live
   handle; `Event::CircuitClosed` carries the id and the exact final
   `CircuitBytes`, on clean close, quota, timeout, error and connection loss
   alike. The accounting boundary is documented in `src/metering.rs`: each
   forwarded byte is counted once (never as both ingress and egress); the
   endpoints' own end-to-end bytes inside the circuit are included; HOP/STOP
   messages and each peer's own hop to the relay are not.
2. **One definition of a byte.** `max_circuit_bytes` is enforced on the same
   counters that are reported, and the "pending data" exchanged with the
   HOP/STOP handshake is both forwarded and counted (upstream forwarded it
   without counting it toward the limit).
3. **No lost relayed dials at startup.** If a client dials through a relay
   while its own dial to that relay is still in flight, the circuit request now
   waits for that connection instead of being cancelled.

`tests/metering.rs` proves the metering against real relayed traffic: per
direction, the relay's counts equal the bytes each endpoint read from and wrote
to its raw circuit stream, across bidirectional transfer, sequential and
concurrent circuits (including several between the same peers), byte-quota and
duration-limit termination, denied circuits, and abrupt peer loss.

Application developers normally do **not** depend on this crate directly. Add
`p2p-net = "0.1.0"` to the application's `Cargo.toml`; Cargo resolves this
companion automatically.
