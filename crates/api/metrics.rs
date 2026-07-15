//! Runtime-owned infrastructure telemetry exposed through `NodeHandle::get_metrics`.
//!
//! These counters live inside the node event loop and are queried through the
//! same command channel as other management operations. That keeps telemetry
//! non-blocking for callers without requiring application code to infer
//! transport-level resource use from payload sizes.

use std::collections::HashMap;

use libp2p::PeerId;

/// Conservative per-message accounting allowance for transport framing,
/// validation, and pubsub metadata around the serialized application payload.
pub(crate) const TRANSPORT_ACCOUNTING_OVERHEAD_BYTES: u64 = 64;

/// Conservative per-side connection setup accounting used when libp2p reports
/// a new connection but does not expose exact handshake byte totals.
pub(crate) const CONNECTION_HANDSHAKE_ESTIMATE_BYTES: u64 = 1536;

#[derive(Debug, Clone, Default)]
pub struct NodeMetrics {
    /// Seconds since this node runtime started.
    pub uptime_seconds: u64,
    /// Accounted ingress/egress bytes observed by the runtime.
    pub bandwidth: BandwidthMetrics,
    /// Node-managed persistence counters.
    pub storage: StorageMetrics,
    /// Runtime pressure and execution counters.
    pub compute: ComputeMetrics,
}

#[derive(Debug, Clone, Default)]
pub struct BandwidthMetrics {
    /// Total accounted egress bytes, including app payloads plus conservative
    /// transport framing/handshake estimates where exact byte totals are not
    /// exposed by the transport.
    pub total_bytes_sent: u64,
    /// Total accounted ingress bytes, including app payloads plus conservative
    /// transport framing/handshake estimates where exact byte totals are not
    /// exposed by the transport.
    pub total_bytes_received: u64,
    /// Per-peer accounted bandwidth keyed by libp2p `PeerId`.
    pub peer_stats: HashMap<PeerId, PeerBandwidth>,
    /// Per-topic accounted bandwidth for application topics and internal
    /// heartbeat traffic where a topic is known.
    pub topic_stats: HashMap<String, TopicBandwidth>,
}

#[derive(Debug, Clone, Default)]
pub struct PeerBandwidth {
    /// Accounted egress bytes to this peer.
    pub bytes_sent: u64,
    /// Accounted ingress bytes from this peer.
    pub bytes_recv: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TopicBandwidth {
    /// Accounted egress bytes on this topic.
    pub bytes_sent: u64,
    /// Accounted ingress bytes on this topic.
    pub bytes_recv: u64,
}

#[derive(Debug, Clone, Default)]
pub struct StorageMetrics {
    /// Count of node-managed chunks/write payloads persisted by the runtime.
    pub total_chunks_stored: u64,
    /// Accounted bytes for node-managed chunks/write payloads.
    pub total_bytes_stored: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ComputeMetrics {
    /// Simple monotonic estimate of runtime command/tick work.
    pub execution_cycles_estimated: u64,
    /// Current count of active/pending runtime requests.
    pub active_request_count: u32,
    /// Connections/peers shed by runtime choking or connection-cap policy.
    pub choked_peers_count: u32,
}

impl NodeMetrics {
    /// Return a copy scoped to a single peer to avoid shipping every per-peer
    /// entry to callers that only need one account.
    #[must_use]
    pub fn for_peer(&self, peer_id: Option<PeerId>) -> Self {
        let Some(peer_id) = peer_id else {
            return self.clone();
        };

        let mut scoped = self.clone();
        scoped.bandwidth.peer_stats = self
            .bandwidth
            .peer_stats
            .get(&peer_id)
            .map(|stats| [(peer_id, stats.clone())].into_iter().collect())
            .unwrap_or_default();
        scoped.bandwidth.topic_stats.clear();
        scoped
    }

    pub(crate) fn record_connection_handshake(&mut self, peer_id: PeerId) {
        self.bandwidth
            .record_sent(Some(peer_id), None, CONNECTION_HANDSHAKE_ESTIMATE_BYTES);
        self.bandwidth
            .record_received(Some(peer_id), None, CONNECTION_HANDSHAKE_ESTIMATE_BYTES);
    }

    pub(crate) fn record_storage_write(&mut self, bytes: usize) {
        self.storage.total_chunks_stored = self.storage.total_chunks_stored.saturating_add(1);
        self.storage.total_bytes_stored = self
            .storage
            .total_bytes_stored
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    pub(crate) fn record_choked_peers(&mut self, count: usize) {
        self.compute.choked_peers_count = u32::try_from(count).unwrap_or(u32::MAX);
    }
}

impl BandwidthMetrics {
    pub(crate) fn record_sent(&mut self, peer_id: Option<PeerId>, topic: Option<&str>, bytes: u64) {
        self.total_bytes_sent = self.total_bytes_sent.saturating_add(bytes);
        if let Some(peer_id) = peer_id {
            let peer = self.peer_stats.entry(peer_id).or_default();
            peer.bytes_sent = peer.bytes_sent.saturating_add(bytes);
        }
        if let Some(topic) = topic {
            let topic = self.topic_stats.entry(topic.to_string()).or_default();
            topic.bytes_sent = topic.bytes_sent.saturating_add(bytes);
        }
    }

    pub(crate) fn record_received(
        &mut self,
        peer_id: Option<PeerId>,
        topic: Option<&str>,
        bytes: u64,
    ) {
        self.total_bytes_received = self.total_bytes_received.saturating_add(bytes);
        if let Some(peer_id) = peer_id {
            let peer = self.peer_stats.entry(peer_id).or_default();
            peer.bytes_recv = peer.bytes_recv.saturating_add(bytes);
        }
        if let Some(topic) = topic {
            let topic = self.topic_stats.entry(topic.to_string()).or_default();
            topic.bytes_recv = topic.bytes_recv.saturating_add(bytes);
        }
    }
}

#[must_use]
pub(crate) fn accounted_transport_bytes(serialized_len: usize) -> u64 {
    u64::try_from(serialized_len)
        .unwrap_or(u64::MAX)
        .saturating_add(TRANSPORT_ACCOUNTING_OVERHEAD_BYTES)
}
