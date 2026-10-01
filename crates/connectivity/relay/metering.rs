//! Billing-grade Circuit Relay metering.
//!
//! Byte counts come from the relay data plane itself: `p2p-net-relay` meters
//! each accepted circuit per direction at the point the relay forwards the
//! opaque circuit stream, and uses the very same counters to enforce
//! `max_circuit_bytes`. See `external/libp2p-relay/src/metering.rs` for the
//! exact accounting boundary (each forwarded byte counted once; endpoint
//! end-to-end Noise/Yamux inside the circuit included; HOP/STOP messages and
//! each peer's own hop to the relay excluded).
//!
//! This module only *identifies, aggregates and reports* those facts. It is
//! not a ledger: completed records are kept in a bounded, in-memory window for
//! observability. Durable economic records (receipts, balances, settlement)
//! belong to the layer consuming the [`crate::NodeEvent`] relay circuit events.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use libp2p::PeerId;
use libp2p_relay::{CircuitBytes, CircuitId, CircuitUsage};
use serde::{Deserialize, Serialize};

/// Completed circuits kept for observability (not billing; see module docs).
pub const RECENT_CLOSED_CIRCUITS: usize = 256;

const MAX_CIRCUIT_BYTES_REACHED: &str = "Max circuit bytes reached.";

/// Why a relay circuit ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum RelayCircuitCloseReason {
    /// Both sides finished the stream.
    Completed,
    /// The relay's `max_circuit_bytes` quota was exceeded.
    ByteLimit,
    /// The relay's `max_circuit_duration` elapsed.
    DurationLimit,
    /// A peer's connection to the relay went away.
    PeerDisconnected,
    /// Any other I/O failure on the circuit.
    TransportError(String),
}

impl RelayCircuitCloseReason {
    #[must_use]
    pub fn from_error(error: Option<&std::io::Error>) -> Self {
        let Some(error) = error else {
            return Self::Completed;
        };
        match error.kind() {
            _ if error.to_string() == MAX_CIRCUIT_BYTES_REACHED => Self::ByteLimit,
            std::io::ErrorKind::TimedOut => Self::DurationLimit,
            std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof => Self::PeerDisconnected,
            _ => Self::TransportError(error.to_string()),
        }
    }
}

/// Usage of one relay circuit. Byte counters are monotonic for its lifetime;
/// once `ended_at_ms` is set they are the exact final totals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayCircuitUsage {
    /// Locally unique, never reused while this node runs.
    pub circuit_id: u64,
    pub src_peer_id: String,
    pub dst_peer_id: String,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub bytes_src_to_dst: u64,
    pub bytes_dst_to_src: u64,
    pub close_reason: Option<RelayCircuitCloseReason>,
}

impl RelayCircuitUsage {
    /// Relay transit bytes: each forwarded byte counted once.
    #[must_use]
    pub fn total_transit_bytes(&self) -> u64 {
        self.bytes_src_to_dst.saturating_add(self.bytes_dst_to_src)
    }

    #[must_use]
    pub fn duration_ms(&self) -> Option<u64> {
        self.ended_at_ms
            .map(|ended| ended.saturating_sub(self.started_at_ms))
    }

    fn with_bytes(mut self, bytes: CircuitBytes) -> Self {
        self.bytes_src_to_dst = bytes.src_to_dst;
        self.bytes_dst_to_src = bytes.dst_to_src;
        self
    }
}

#[derive(Debug, Clone)]
struct ActiveCircuit {
    record: RelayCircuitUsage,
    usage: Arc<CircuitUsage>,
    reported: CircuitBytes,
}

/// Per-circuit relay usage plus the aggregate derived from it.
#[derive(Debug, Default, Clone)]
pub struct RelayMeter {
    active: HashMap<u64, ActiveCircuit>,
    closed_src_to_dst: u64,
    closed_dst_to_src: u64,
    closed_circuits: u64,
    recent_closed: VecDeque<RelayCircuitUsage>,
}

impl RelayMeter {
    /// Start metering an accepted circuit from its live data-plane counters.
    pub fn open(
        &mut self,
        circuit_id: CircuitId,
        src: PeerId,
        dst: PeerId,
        usage: Arc<CircuitUsage>,
        now_ms: u64,
    ) -> RelayCircuitUsage {
        let record = RelayCircuitUsage {
            circuit_id: circuit_id.get(),
            src_peer_id: src.to_string(),
            dst_peer_id: dst.to_string(),
            started_at_ms: now_ms,
            ended_at_ms: None,
            bytes_src_to_dst: 0,
            bytes_dst_to_src: 0,
            close_reason: None,
        }
        .with_bytes(usage.snapshot());
        let reported = usage.snapshot();
        self.active.insert(
            circuit_id.get(),
            ActiveCircuit {
                record: record.clone(),
                usage,
                reported,
            },
        );
        record
    }

    /// Finish a circuit with the relay's exact final totals.
    pub fn close(
        &mut self,
        circuit_id: CircuitId,
        src: PeerId,
        dst: PeerId,
        bytes: CircuitBytes,
        error: Option<&std::io::Error>,
        now_ms: u64,
    ) -> RelayCircuitUsage {
        let started_at_ms = self
            .active
            .remove(&circuit_id.get())
            .map_or(now_ms, |active| active.record.started_at_ms);
        let record = RelayCircuitUsage {
            circuit_id: circuit_id.get(),
            src_peer_id: src.to_string(),
            dst_peer_id: dst.to_string(),
            started_at_ms,
            ended_at_ms: Some(now_ms),
            bytes_src_to_dst: bytes.src_to_dst,
            bytes_dst_to_src: bytes.dst_to_src,
            close_reason: Some(RelayCircuitCloseReason::from_error(error)),
        };
        self.closed_src_to_dst = self.closed_src_to_dst.saturating_add(bytes.src_to_dst);
        self.closed_dst_to_src = self.closed_dst_to_src.saturating_add(bytes.dst_to_src);
        self.closed_circuits = self.closed_circuits.saturating_add(1);
        if self.recent_closed.len() == RECENT_CLOSED_CIRCUITS {
            self.recent_closed.pop_front();
        }
        self.recent_closed.push_back(record.clone());
        record
    }

    /// Active circuits whose usage changed since the last call (coalesced
    /// progress reporting; never one event per packet).
    pub fn take_changed(&mut self) -> Vec<RelayCircuitUsage> {
        let mut changed = Vec::new();
        for active in self.active.values_mut() {
            let now = active.usage.snapshot();
            if now != active.reported {
                active.reported = now;
                changed.push(active.record.clone().with_bytes(now));
            }
        }
        changed.sort_by_key(|usage| usage.circuit_id);
        changed
    }

    /// Aggregate relay transit from the same per-circuit counters: completed
    /// circuits' final totals plus active circuits' live totals.
    #[must_use]
    pub fn totals(&self) -> CircuitBytes {
        let mut totals = CircuitBytes {
            src_to_dst: self.closed_src_to_dst,
            dst_to_src: self.closed_dst_to_src,
        };
        for active in self.active.values() {
            let live = active.usage.snapshot();
            totals.src_to_dst = totals.src_to_dst.saturating_add(live.src_to_dst);
            totals.dst_to_src = totals.dst_to_src.saturating_add(live.dst_to_src);
        }
        totals
    }

    #[must_use]
    pub fn active_circuits(&self) -> Vec<RelayCircuitUsage> {
        let mut active: Vec<_> = self
            .active
            .values()
            .map(|active| active.record.clone().with_bytes(active.usage.snapshot()))
            .collect();
        active.sort_by_key(|usage| usage.circuit_id);
        active
    }

    #[must_use]
    pub fn recent_closed(&self) -> Vec<RelayCircuitUsage> {
        self.recent_closed.iter().cloned().collect()
    }

    #[must_use]
    pub fn closed_circuits(&self) -> u64 {
        self.closed_circuits
    }
}

/// Relay usage view for snapshots, metrics and dashboards (observability,
/// not a billing ledger).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayUsageSnapshot {
    pub bytes_src_to_dst: u64,
    pub bytes_dst_to_src: u64,
    pub completed_circuits: u64,
    pub active_circuits: Vec<RelayCircuitUsage>,
    pub recent_closed_circuits: Vec<RelayCircuitUsage>,
}

impl super::RelayState {
    #[must_use]
    pub fn relay_usage_snapshot(&self) -> RelayUsageSnapshot {
        RelayUsageSnapshot {
            bytes_src_to_dst: self.relay_bytes_src_to_dst,
            bytes_dst_to_src: self.relay_bytes_dst_to_src,
            completed_circuits: self.relay_meter.closed_circuits(),
            active_circuits: self.relay_meter.active_circuits(),
            recent_closed_circuits: self.relay_meter.recent_closed(),
        }
    }

    /// Re-derive the aggregate relay counters from the per-circuit meter.
    pub(crate) fn refresh_relay_totals(&mut self) {
        let totals = self.relay_meter.totals();
        self.relay_bytes_src_to_dst = totals.src_to_dst;
        self.relay_bytes_dst_to_src = totals.dst_to_src;
        self.relay_bytes_forwarded = totals.total();
    }
}

#[cfg(test)]
mod tests;
