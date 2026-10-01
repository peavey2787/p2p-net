# Relay metering

p2p-net measures, per Circuit Relay circuit, exactly how many bytes a relay node
forwarded in each direction. The numbers come from the relay data plane, are the
same numbers the relay's `max_circuit_bytes` quota is enforced on, and are
exposed as events, snapshot fields and metrics. A higher layer (for example an
AOL2 relay marketplace) can price, authorize, receipt and settle relay usage on
top of them without touching or re-deriving anything inside the relay.

## Where the bytes are counted

The relay copy loop in the `p2p-net-relay` companion (`external/libp2p-relay`)
counts each byte when it successfully writes it to the far side of the circuit:

| Counter | Meaning |
| --- | --- |
| `bytes_src_to_dst` | Bytes read from the circuit source (the peer that sent HOP `CONNECT`) and written to the destination |
| `bytes_dst_to_src` | Bytes read from the destination and written to the source |
| total transit | `bytes_src_to_dst + bytes_dst_to_src` |

**Each forwarded byte is counted once.** 900 MB A→B plus 300 MB B→A through a
relay is 1.2 GB of relay transit, not 2.4 GB: ingress and egress of the same
byte are never both counted.

**Included:** everything the two endpoints exchange on the circuit stream. The
relay cannot see inside the circuit, so the endpoints' own end-to-end
encryption and multiplexing (Noise/Yamux of the relayed connection) is transit
like any payload. Bytes either peer sent alongside the HOP/STOP handshake
("pending data") are forwarded on the circuit and are included.

**Excluded:** Circuit Relay protocol messages (HOP/STOP protobufs) and the
framing of each peer's own connection to the relay (that hop's
TCP/QUIC/Noise/Yamux). Those are not forwarded payload.

For a circuit that closes cleanly, `bytes_src_to_dst` equals exactly what the
destination read from its raw circuit stream, and vice versa. If a peer
disappears mid-transfer, the counters still hold exactly what the relay wrote
toward it, which can exceed what that peer managed to read.

The boundary is pinned by tests (see below); changing it is a breaking change
for anyone billing on these numbers.

## Quota agreement

`max_circuit_bytes` is checked against the same per-circuit total. When a
circuit is cut because it exceeded its byte quota, its reported final bytes are
the bytes that were counted toward that quota. There is no second definition
of a byte for observability.

## API

Relay nodes emit, through `NodeHandle::subscribe_events()`:

- `NodeEvent::RelayCircuitOpened { circuit_id, src_peer_id, dst_peer_id }`
- `NodeEvent::RelayCircuitUsage { circuit_id, bytes_src_to_dst, bytes_dst_to_src }`:
  coalesced progress, at most once per second per circuit and only when the
  numbers changed (never per packet)
- `NodeEvent::RelayCircuitClosed { circuit_id, src_peer_id, dst_peer_id,
  bytes_src_to_dst, bytes_dst_to_src, duration_ms, close_reason }`, whose byte
  totals are exact and final. `close_reason` is one of `completed`,
  `byte_limit`, `duration_limit`, `peer_disconnected` or `transport_error`.

`circuit_id` is unique and never reused while the node runs. Circuits between
the same two peers are separate circuits with separate ids and counters. A
circuit that was denied or never established is never opened, so it never
accrues bytes.

The snapshot (`NodeSnapshot`) exposes `relay_bytes_forwarded` (total transit)
and `relay_usage`: per-direction totals, the count of completed circuits, live
per-circuit usage, and a bounded window of recently closed circuits.
Prometheus exports `p2p_relay_bytes_forwarded`, `p2p_relay_bytes_src_to_dst`,
`p2p_relay_bytes_dst_to_src` and `p2p_relay_circuits_completed`. All of these
aggregates are derived from the per-circuit counters (completed circuits' final
totals plus active circuits' live totals); there is no separate estimate.

## Persistence boundary

p2p-net provides exact measurements and events. It does **not** keep billing
records: the snapshot's recent-circuit window is bounded observability that
drops old entries. Durable economic records (usage receipts, balances,
settlement) are the responsibility of the layer consuming
`RelayCircuitClosed`, which should persist each event it receives.

## Tests

- `external/libp2p-relay/tests/metering.rs`: real relay circuits with byte
  counters on the endpoints' raw circuit streams; asserts exact per-direction
  equality for bidirectional transfer, sequential circuits summing to the
  aggregate, concurrent circuits between the same peers isolated by id, byte
  quota and duration-limit termination, denied circuits, and abrupt peer loss.
- `qa/tests/relay/relay_metering.rs`: real p2p-net nodes (relay plus two
  clients that can only reach each other through it) exchange application
  messages; asserts the relay node's circuit events, snapshot and Prometheus
  output agree with each other and cover the traffic sent.
