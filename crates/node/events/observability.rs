//! Batched observability counters and pulses, flushed into the snapshot.

use std::collections::VecDeque;

use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::DhtProviderState;
use crate::connectivity::peer_book::PeerBook;

use super::super::dial::AutoDialStats;
use super::super::snapshot::NodeSnapshot;
use super::sync_peer_connectivity_fields;

#[derive(Debug, Default)]
pub(crate) struct ObservabilityBatch {
    app_messages_received: usize,
    app_messages_ignored: usize,
    app_messages_rejected: usize,
    gossip_messages_accepted: usize,
    gossip_messages_ignored: usize,
    gossip_messages_rejected: usize,
    peer_connectivity_dirty: bool,
    dht_snapshot_dirty: bool,
    pulses: VecDeque<String>,
}

impl ObservabilityBatch {
    const MAX_PENDING_PULSES: usize = 64;

    pub(crate) fn app_received(&mut self) {
        self.app_messages_received = self.app_messages_received.saturating_add(1);
    }

    pub(crate) fn app_ignored(&mut self) {
        self.app_messages_ignored = self.app_messages_ignored.saturating_add(1);
    }

    pub(crate) fn app_rejected(&mut self) {
        self.app_messages_rejected = self.app_messages_rejected.saturating_add(1);
    }

    pub(crate) fn gossip_accepted(&mut self, peer_connectivity_dirty: bool) {
        self.gossip_messages_accepted = self.gossip_messages_accepted.saturating_add(1);
        self.peer_connectivity_dirty |= peer_connectivity_dirty;
    }

    pub(crate) fn gossip_ignored(&mut self) {
        self.gossip_messages_ignored = self.gossip_messages_ignored.saturating_add(1);
    }

    pub(crate) fn gossip_rejected(&mut self) {
        self.gossip_messages_rejected = self.gossip_messages_rejected.saturating_add(1);
    }

    pub(crate) fn dht_dirty(&mut self) {
        self.dht_snapshot_dirty = true;
    }

    pub(crate) fn peer_connectivity_dirty(&mut self) {
        self.peer_connectivity_dirty = true;
    }

    pub(crate) fn pulse(&mut self, line: String) {
        if self.pulses.len() >= Self::MAX_PENDING_PULSES {
            let _ = self.pulses.pop_front();
        }
        self.pulses.push_back(line);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.app_messages_received == 0
            && self.app_messages_ignored == 0
            && self.app_messages_rejected == 0
            && self.gossip_messages_accepted == 0
            && self.gossip_messages_ignored == 0
            && self.gossip_messages_rejected == 0
            && !self.peer_connectivity_dirty
            && !self.dht_snapshot_dirty
            && self.pulses.is_empty()
    }
}

pub(crate) fn flush_observability_snapshot(
    snapshot: &mut NodeSnapshot,
    batch: &mut ObservabilityBatch,
    dht_state: &DhtProviderState,
    peer_book: &PeerBook,
    auto_dial_stats: &AutoDialStats,
    pending_connections: &PendingConnectionPlans,
    auto_connect_enabled: bool,
) {
    snapshot.app_messages_received = snapshot
        .app_messages_received
        .saturating_add(batch.app_messages_received);
    snapshot.app_messages_ignored = snapshot
        .app_messages_ignored
        .saturating_add(batch.app_messages_ignored);
    snapshot.app_messages_rejected = snapshot
        .app_messages_rejected
        .saturating_add(batch.app_messages_rejected);
    snapshot.gossip_messages_accepted = snapshot
        .gossip_messages_accepted
        .saturating_add(batch.gossip_messages_accepted);
    snapshot.gossip_messages_ignored = snapshot
        .gossip_messages_ignored
        .saturating_add(batch.gossip_messages_ignored);
    snapshot.gossip_messages_rejected = snapshot
        .gossip_messages_rejected
        .saturating_add(batch.gossip_messages_rejected);
    if batch.dht_snapshot_dirty {
        snapshot.dht_provider_announce_attempts = dht_state.announce_attempts;
        snapshot.dht_provider_announce_failures = dht_state.announce_failures;
        snapshot.dht_provider_namespaces_announced = dht_state.namespaces_announced.len();
        snapshot.dht_provider_queries = dht_state.provider_queries;
        snapshot.dht_provider_query_failures = dht_state.provider_query_failures;
        snapshot.dht_provider_records_found = dht_state.provider_records_found;
        snapshot.dht_provider_queries_finished = dht_state.provider_queries_finished;
        snapshot.dht_provider_peers_discovered = dht_state.provider_peer_count();
    }
    if batch.peer_connectivity_dirty {
        sync_peer_connectivity_fields(
            snapshot,
            peer_book,
            auto_dial_stats,
            pending_connections,
            auto_connect_enabled,
        );
    }
    for line in batch.pulses.drain(..) {
        super::super::push_pulse(&mut snapshot.pulses, line);
    }
    *batch = ObservabilityBatch::default();
}
