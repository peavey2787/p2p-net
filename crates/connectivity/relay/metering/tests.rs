use std::io::{Error, ErrorKind};
use std::sync::Arc;

use libp2p::PeerId;
use libp2p_relay::{CircuitBytes, CircuitUsage};

use super::*;

fn circuit(meter: &mut RelayMeter, id: u64, now_ms: u64) -> (CircuitId, PeerId, PeerId) {
    let ids = circuit_ids(id);
    let (src, dst) = (PeerId::random(), PeerId::random());
    meter.open(ids, src, dst, Arc::new(CircuitUsage::default()), now_ms);
    (ids, src, dst)
}

/// `CircuitId` has no public constructor; mint ids the way the relay does.
fn circuit_ids(id: u64) -> CircuitId {
    let mut circuit = CircuitId::default();
    for _ in 0..id {
        circuit = circuit + 1;
    }
    circuit
}

#[test]
fn close_records_exact_final_totals_duration_and_reason() {
    let mut meter = RelayMeter::default();
    let (id, src, dst) = circuit(&mut meter, 3, 1_000);
    assert_eq!(meter.active_circuits().len(), 1);

    let bytes = CircuitBytes {
        src_to_dst: 900,
        dst_to_src: 300,
    };
    let record = meter.close(id, src, dst, bytes, None, 4_500);
    assert_eq!(record.circuit_id, 3);
    assert_eq!(record.src_peer_id, src.to_string());
    assert_eq!(record.dst_peer_id, dst.to_string());
    assert_eq!(record.bytes_src_to_dst, 900);
    assert_eq!(record.bytes_dst_to_src, 300);
    assert_eq!(
        record.total_transit_bytes(),
        1_200,
        "counted once, not 2400"
    );
    assert_eq!(record.duration_ms(), Some(3_500));
    assert_eq!(
        record.close_reason,
        Some(RelayCircuitCloseReason::Completed)
    );
    assert!(meter.active_circuits().is_empty());
    assert_eq!(meter.recent_closed(), vec![record]);
    assert_eq!(meter.closed_circuits(), 1);
}

#[test]
fn aggregate_is_the_sum_of_circuit_totals() {
    let mut meter = RelayMeter::default();
    let mut expected = CircuitBytes::default();
    for (id, (a, b)) in [(1u64, (10u64, 1u64)), (2, (20, 2)), (3, (30, 3))] {
        let (cid, src, dst) = circuit(&mut meter, id, 0);
        let bytes = CircuitBytes {
            src_to_dst: a,
            dst_to_src: b,
        };
        meter.close(cid, src, dst, bytes, None, 1);
        expected.src_to_dst += a;
        expected.dst_to_src += b;
    }
    assert_eq!(meter.totals(), expected);
    assert_eq!(meter.totals().total(), 66);
}

#[test]
fn close_reasons_follow_the_relay_error() {
    let reason = |error: Error| RelayCircuitCloseReason::from_error(Some(&error));
    assert_eq!(
        reason(Error::other("Max circuit bytes reached.")),
        RelayCircuitCloseReason::ByteLimit
    );
    assert_eq!(
        reason(ErrorKind::TimedOut.into()),
        RelayCircuitCloseReason::DurationLimit
    );
    assert_eq!(
        reason(ErrorKind::ConnectionAborted.into()),
        RelayCircuitCloseReason::PeerDisconnected
    );
    assert!(matches!(
        reason(Error::other("boom")),
        RelayCircuitCloseReason::TransportError(_)
    ));
    assert_eq!(
        RelayCircuitCloseReason::from_error(None),
        RelayCircuitCloseReason::Completed
    );
}

#[test]
fn unchanged_circuits_produce_no_progress_events() {
    let mut meter = RelayMeter::default();
    circuit(&mut meter, 1, 0);
    assert!(
        meter.take_changed().is_empty(),
        "no forwarded bytes, nothing to report"
    );
}

#[test]
fn recent_closed_window_is_bounded() {
    let mut meter = RelayMeter::default();
    for id in 0..(RECENT_CLOSED_CIRCUITS as u64 + 5) {
        let (cid, src, dst) = circuit(&mut meter, id, 0);
        meter.close(cid, src, dst, CircuitBytes::default(), None, 0);
    }
    assert_eq!(meter.recent_closed().len(), RECENT_CLOSED_CIRCUITS);
    assert_eq!(meter.closed_circuits(), RECENT_CLOSED_CIRCUITS as u64 + 5);
    assert_eq!(
        meter.recent_closed()[0].circuit_id,
        5,
        "oldest records drop first"
    );
}
