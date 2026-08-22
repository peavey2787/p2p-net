//! Unified in-memory peer book for the public application API.
//!
//! Discovery sources update this structure with peer ids, addresses, namespaces,
//! capability hints, and connection state. `get_peers()` reads from this single
//! source instead of peeking only at the live swarm connection set.

use std::collections::{BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use libp2p::{Multiaddr, PeerId};

use crate::api::{PeerInfo, PeerSource};

const MAX_ADDRS_PER_PEER: usize = 64;
pub const DEFAULT_MAX_PEER_BOOK_RECORDS: usize = 2048;

/// One normalized peer record in the local peer book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRecord {
    pub peer_id: PeerId,
    pub addresses: BTreeSet<String>,
    pub sources: BTreeSet<PeerSource>,
    pub namespaces: BTreeSet<String>,
    pub connected: bool,
    pub supports_relay: Option<bool>,
    pub supports_rendezvous: Option<bool>,
    pub supports_dcutr: Option<bool>,
    pub relay_preferred: bool,
    pub last_seen_unix_secs: Option<u64>,
    pub failures: u32,
}

impl PeerRecord {
    #[must_use]
    pub fn new(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            addresses: BTreeSet::new(),
            sources: BTreeSet::new(),
            namespaces: BTreeSet::new(),
            connected: false,
            supports_relay: None,
            supports_rendezvous: None,
            supports_dcutr: None,
            relay_preferred: false,
            last_seen_unix_secs: None,
            failures: 0,
        }
    }

    fn mark_seen(&mut self) {
        self.last_seen_unix_secs = Some(now_unix_secs());
    }

    #[must_use]
    pub fn to_peer_info(&self) -> PeerInfo {
        let namespace = if self.namespaces.len() == 1 {
            self.namespaces.iter().next().cloned()
        } else {
            None
        };
        PeerInfo {
            peer_id: self.peer_id.to_string(),
            connected: self.connected,
            addresses: self.addresses.iter().cloned().collect(),
            sources: self.sources.iter().copied().collect(),
            supports_relay: self.supports_relay,
            supports_rendezvous: self.supports_rendezvous,
            supports_dcutr: self.supports_dcutr,
            last_seen_unix_secs: self.last_seen_unix_secs,
            namespace,
        }
    }
}

/// Local in-memory peer index merged from all discovery and connection sources.
///
/// Disconnected records are bounded and oldest-first evicted. Connected peers are
/// protected from eviction; connection limits provide their independent bound.
/// An auxiliary index makes periodic reconnect candidate selection O(candidates)
/// instead of repeatedly scanning every known peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerBook {
    peers: HashMap<PeerId, PeerRecord>,
    disconnected_namespace_peers: BTreeSet<PeerId>,
    disconnected_eviction_order: BTreeSet<(u64, PeerId)>,
    connected_count: usize,
    max_records: usize,
}

impl Default for PeerBook {
    fn default() -> Self {
        Self::with_max_records(DEFAULT_MAX_PEER_BOOK_RECORDS)
    }
}

impl PeerBook {
    #[must_use]
    pub fn with_max_records(max_records: usize) -> Self {
        Self {
            peers: HashMap::new(),
            disconnected_namespace_peers: BTreeSet::new(),
            disconnected_eviction_order: BTreeSet::new(),
            connected_count: 0,
            max_records: max_records.max(1),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    pub fn record_peer(&mut self, peer_id: PeerId, source: PeerSource) {
        self.mutate_peer(peer_id, |record| {
            record.sources.insert(source);
        });
    }

    pub fn record_addr(&mut self, peer_id: PeerId, addr: Multiaddr, source: PeerSource) {
        self.mutate_peer(peer_id, |record| {
            record.sources.insert(source);
            record.addresses.insert(addr.to_string());
            prune_peer_addrs(record);
        });
    }

    pub fn record_namespace(
        &mut self,
        peer_id: PeerId,
        namespace: impl Into<String>,
        source: PeerSource,
    ) {
        let namespace = namespace.into();
        self.mutate_peer(peer_id, move |record| {
            record.sources.insert(source);
            record.namespaces.insert(namespace);
        });
    }

    pub fn record_connected(&mut self, peer_id: PeerId, addr: Option<Multiaddr>) {
        self.mutate_peer(peer_id, |record| {
            record.connected = true;
            record.sources.insert(PeerSource::Connected);
            if let Some(addr) = addr {
                record.addresses.insert(addr.to_string());
                prune_peer_addrs(record);
            }
        });
    }

    pub fn record_capabilities(
        &mut self,
        peer_id: PeerId,
        supports_relay: Option<bool>,
        supports_rendezvous: Option<bool>,
        supports_dcutr: Option<bool>,
    ) {
        self.mutate_peer(peer_id, |record| {
            if supports_relay.is_some() {
                record.supports_relay = supports_relay;
            }
            if supports_rendezvous.is_some() {
                record.supports_rendezvous = supports_rendezvous;
            }
            if supports_dcutr.is_some() {
                record.supports_dcutr = supports_dcutr;
            }
        });
    }

    pub fn record_relay_preferred(&mut self, peer_id: PeerId, relay_preferred: bool) {
        self.mutate_peer(peer_id, |record| {
            record.relay_preferred = relay_preferred;
        });
    }

    pub fn record_disconnected(&mut self, peer_id: PeerId) {
        self.mutate_peer(peer_id, |record| {
            record.connected = false;
        });
    }

    pub fn record_disconnected_if_known(&mut self, peer_id: PeerId) {
        if !self.peers.contains_key(&peer_id) {
            return;
        }
        self.mutate_peer(peer_id, |record| {
            record.connected = false;
        });
    }

    pub fn record_failure(&mut self, peer_id: PeerId) {
        self.mutate_peer(peer_id, |record| {
            record.failures = record.failures.saturating_add(1);
        });
    }

    #[must_use]
    pub fn record(&self, peer_id: &PeerId) -> Option<&PeerRecord> {
        self.peers.get(peer_id)
    }

    pub fn records(&self) -> impl Iterator<Item = &PeerRecord> {
        self.peers.values()
    }

    /// Indexed disconnected peers that have at least one application namespace.
    pub fn reconnect_candidates(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.disconnected_namespace_peers.iter().copied()
    }

    #[must_use]
    pub fn has_application_namespace(&self, peer_id: &PeerId, namespaces: &[String]) -> bool {
        self.record(peer_id).is_some_and(|record| {
            record
                .namespaces
                .iter()
                .any(|namespace| namespaces.iter().any(|candidate| candidate == namespace))
        })
    }

    #[must_use]
    pub fn peers(&self) -> Vec<PeerInfo> {
        let mut peers = self
            .peers
            .values()
            .map(PeerRecord::to_peer_info)
            .collect::<Vec<_>>();
        peers.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
        peers
    }

    #[must_use]
    pub fn connected_count(&self) -> usize {
        self.connected_count
    }

    #[must_use]
    pub fn discovered_count(&self) -> usize {
        self.peers.len().saturating_sub(self.connected_count)
    }

    fn mutate_peer(&mut self, peer_id: PeerId, mutate: impl FnOnce(&mut PeerRecord)) {
        let previous = self
            .peers
            .get(&peer_id)
            .map(|record| (record.connected, record.last_seen_unix_secs));
        if let Some((false, Some(last_seen))) = previous {
            self.disconnected_eviction_order
                .remove(&(last_seen, peer_id));
        }

        {
            let record = self
                .peers
                .entry(peer_id)
                .or_insert_with(|| PeerRecord::new(peer_id));
            mutate(record);
            record.mark_seen();
        }
        self.sync_indexes(
            peer_id,
            previous.map(|(connected, _)| connected).unwrap_or(false),
        );
        self.enforce_record_bound();
    }

    fn sync_indexes(&mut self, peer_id: PeerId, was_connected: bool) {
        let Some(record) = self.peers.get(&peer_id) else {
            return;
        };
        if was_connected != record.connected {
            if record.connected {
                self.connected_count = self.connected_count.saturating_add(1);
            } else {
                self.connected_count = self.connected_count.saturating_sub(1);
            }
        }
        if !record.connected && !record.namespaces.is_empty() {
            self.disconnected_namespace_peers.insert(peer_id);
        } else {
            self.disconnected_namespace_peers.remove(&peer_id);
        }
        if !record.connected {
            if let Some(last_seen) = record.last_seen_unix_secs {
                self.disconnected_eviction_order
                    .insert((last_seen, peer_id));
            }
        }
    }

    fn enforce_record_bound(&mut self) {
        while self.peers.len() > self.max_records {
            let victim = self.disconnected_eviction_order.iter().next().copied();
            let Some((last_seen, victim)) = victim else {
                // Connected records are protected; connection limits are their bound.
                break;
            };
            self.disconnected_eviction_order
                .remove(&(last_seen, victim));
            self.peers.remove(&victim);
            self.disconnected_namespace_peers.remove(&victim);
        }
    }
}

fn prune_peer_addrs(record: &mut PeerRecord) {
    while record.addresses.len() > MAX_ADDRS_PER_PEER {
        let Some(oldest_sorted) = record.addresses.iter().next().cloned() else {
            break;
        };
        record.addresses.remove(&oldest_sorted);
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_book_merges_sources_for_same_peer() {
        let peer = PeerId::random();
        let addr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{peer}")
            .parse::<Multiaddr>()
            .expect("multiaddr");
        let mut book = PeerBook::default();
        book.record_peer(peer, PeerSource::Bootstrap);
        book.record_addr(peer, addr, PeerSource::PeerCache);
        book.record_namespace(peer, "p2p-net/1/app/tag", PeerSource::DhtProvider);
        book.record_capabilities(peer, Some(true), None, Some(true));
        book.record_relay_preferred(peer, true);
        book.record_connected(peer, None);

        let peers = book.peers();
        assert_eq!(peers.len(), 1);
        assert!(peers[0].connected);
        assert!(peers[0].has_source(PeerSource::Bootstrap));
        assert!(peers[0].has_source(PeerSource::PeerCache));
        assert!(peers[0].has_source(PeerSource::DhtProvider));
        assert_eq!(peers[0].namespace.as_deref(), Some("p2p-net/1/app/tag"));
        assert_eq!(peers[0].supports_relay, Some(true));
        assert_eq!(peers[0].supports_dcutr, Some(true));
        assert!(book.record(&peer).expect("record").relay_preferred);
        assert_eq!(book.connected_count(), 1);
        assert_eq!(book.reconnect_candidates().count(), 0);
    }

    #[test]
    fn disconnected_unknown_peer_does_not_create_record() {
        let mut book = PeerBook::default();
        book.record_disconnected_if_known(PeerId::random());

        assert!(book.is_empty());
    }

    #[test]
    fn peer_record_addresses_are_bounded() {
        let peer = PeerId::random();
        let mut book = PeerBook::default();
        for port in 10_000..10_000 + MAX_ADDRS_PER_PEER + 5 {
            let addr = format!("/ip4/203.0.113.1/tcp/{port}/p2p/{peer}")
                .parse::<Multiaddr>()
                .expect("multiaddr");
            book.record_addr(peer, addr, PeerSource::DhtProvider);
        }

        assert_eq!(
            book.record(&peer).expect("record").addresses.len(),
            MAX_ADDRS_PER_PEER
        );
    }

    #[test]
    fn disconnected_records_are_bounded_and_indexed() {
        let mut book = PeerBook::with_max_records(4);
        for _ in 0..8 {
            let peer = PeerId::random();
            book.record_namespace(peer, "p2p-net/1/app/tag", PeerSource::DhtProvider);
        }

        assert_eq!(book.len(), 4);
        assert_eq!(book.discovered_count(), 4);
        assert_eq!(book.reconnect_candidates().count(), 4);
    }

    #[test]
    fn connected_records_are_protected_from_discovery_eviction() {
        let connected = PeerId::random();
        let mut book = PeerBook::with_max_records(2);
        book.record_connected(connected, None);
        for _ in 0..5 {
            book.record_peer(PeerId::random(), PeerSource::DhtProvider);
        }

        assert!(book.record(&connected).is_some());
        assert_eq!(book.connected_count(), 1);
        assert!(book.len() <= 2);
    }
}
