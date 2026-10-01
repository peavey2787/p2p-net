//! Exact per-circuit relay metering (p2p-net addition to libp2p-relay 0.22.0).
//!
//! # Accounting boundary
//!
//! A circuit's usage is the number of **logical stream bytes the relay wrote
//! to the opposite side of the circuit**, counted once per direction:
//!
//! - `src_to_dst`: bytes read from the source (the peer that sent HOP
//!   `CONNECT`) and written to the destination's STOP stream;
//! - `dst_to_src`: bytes read from the destination and written to the source.
//!
//! Each forwarded byte is counted exactly once, when the relay successfully
//! writes it to the far side. Ingress and egress are never both counted, so
//! `A -> relay -> B` of 900 bytes and `B -> relay -> A` of 300 bytes is 1200
//! bytes of transit, not 2400.
//!
//! **Included:** every byte the endpoints exchange on the circuit stream. The
//! relay cannot see inside it, so the endpoints' own end-to-end security and
//! multiplexing (e.g. Noise/Yamux of the relayed connection) is billed transit
//! like any other payload. Bytes either peer sent alongside the HOP/STOP
//! handshake ("pending data") are forwarded on the circuit and are included.
//!
//! **Excluded:** the Circuit Relay protocol messages themselves (HOP/STOP
//! protobufs) and the framing of each peer's own connection *to the relay*
//! (that hop's Noise/Yamux/TCP/QUIC). Those are not forwarded payload.
//!
//! For a circuit that closes cleanly, `src_to_dst` therefore equals the bytes
//! the destination read from its raw circuit stream, and vice versa. If a peer
//! vanishes mid-transfer, the counters still hold exactly what the relay wrote
//! toward it, which may exceed what that peer managed to read.
//!
//! The same counters drive `max_circuit_bytes` enforcement, so the bytes a
//! circuit is billed for are exactly the bytes counted toward its quota.
//! Counters are monotonic and saturate at `u64::MAX` instead of wrapping.

use std::sync::atomic::{AtomicU64, Ordering};

/// Live, shared byte counters for one relay circuit.
///
/// The relay data path writes to these as it forwards bytes; observers may
/// read them at any time to report in-progress usage.
#[derive(Debug, Default)]
pub struct CircuitUsage {
    src_to_dst: AtomicU64,
    dst_to_src: AtomicU64,
}

impl CircuitUsage {
    /// Bytes forwarded from the circuit source to its destination so far.
    #[must_use]
    pub fn src_to_dst(&self) -> u64 {
        self.src_to_dst.load(Ordering::Acquire)
    }

    /// Bytes forwarded from the circuit destination to its source so far.
    #[must_use]
    pub fn dst_to_src(&self) -> u64 {
        self.dst_to_src.load(Ordering::Acquire)
    }

    /// Point-in-time copy of both directions.
    #[must_use]
    pub fn snapshot(&self) -> CircuitBytes {
        CircuitBytes {
            src_to_dst: self.src_to_dst(),
            dst_to_src: self.dst_to_src(),
        }
    }

    pub(crate) fn record_src_to_dst(&self, bytes: u64) {
        saturating_add(&self.src_to_dst, bytes);
    }

    pub(crate) fn record_dst_to_src(&self, bytes: u64) {
        saturating_add(&self.dst_to_src, bytes);
    }
}

/// Byte totals of one relay circuit, per direction.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CircuitBytes {
    pub src_to_dst: u64,
    pub dst_to_src: u64,
}

impl CircuitBytes {
    /// Total relay transit: each forwarded byte counted once.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.src_to_dst.saturating_add(self.dst_to_src)
    }
}

fn saturating_add(counter: &AtomicU64, bytes: u64) {
    // Single writer per direction (the circuit's copy future), but use an
    // atomic update so a reader never observes a torn or wrapped value.
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_add(bytes))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_are_independent_and_total_counts_each_byte_once() {
        let usage = CircuitUsage::default();
        usage.record_src_to_dst(900);
        usage.record_dst_to_src(300);
        usage.record_src_to_dst(100);
        let bytes = usage.snapshot();
        assert_eq!(bytes.src_to_dst, 1_000);
        assert_eq!(bytes.dst_to_src, 300);
        assert_eq!(bytes.total(), 1_300);
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let usage = CircuitUsage::default();
        usage.record_src_to_dst(u64::MAX - 1);
        usage.record_src_to_dst(10);
        usage.record_dst_to_src(u64::MAX);
        assert_eq!(usage.src_to_dst(), u64::MAX);
        assert_eq!(usage.snapshot().total(), u64::MAX);
    }
}
