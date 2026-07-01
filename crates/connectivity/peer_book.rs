//! Unified in-memory peer book for the public six-primitives API.
//!
//! Discovery sources update this structure with peer ids, addresses, namespaces,
//! capability hints, and connection state. `get_peers()` reads from this single
//! source instead of peeking only at the live swarm connection set.

use std::collections::{BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use libp2p::{Multiaddr, PeerId};
use crate::api::{PeerInfo, PeerSource};

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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerBook {
    peers: HashMap<PeerId, PeerRecord>,
}

impl PeerBook {
    #[must_use]
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    pub fn record_peer(&mut self, peer_id: PeerId, source: PeerSource) {
        let record = self
            .peers
            .entry(peer_id)
            .or_insert_with(|| PeerRecord::new(peer_id));
        record.sources.insert(source);
        record.mark_seen();
    }

    pub fn record_addr(&mut self, peer_id: PeerId, addr: Multiaddr, source: PeerSource) {
        let record = self
            .peers
            .entry(peer_id)
            .or_insert_with(|| PeerRecord::new(peer_id));
        record.sources.insert(source);
        record.addresses.insert(addr.to_string());
        record.mark_seen();
    }

    pub fn record_namespace(&mut self, peer_id: PeerId, namespace: impl Into<String>, source: PeerSource) {
        let record = self
            .peers
            .entry(peer_id)
            .or_insert_with(|| PeerRecord::new(peer_id));
        record.sources.insert(source);
        record.namespaces.insert(namespace.into());
        record.mark_seen();
    }

    pub fn record_connected(&mut self, peer_id: PeerId, addr: Option<Multiaddr>) {
        let record = self
            .peers
            .entry(peer_id)
            .or_insert_with(|| PeerRecord::new(peer_id));
        record.connected = true;
        record.sources.insert(PeerSource::Connected);
        if let Some(addr) = addr {
            record.addresses.insert(addr.to_string());
        }
        record.mark_seen();
    }

    pub fn record_disconnected(&mut self, peer_id: PeerId) {
        let record = self
            .peers
            .entry(peer_id)
            .or_insert_with(|| PeerRecord::new(peer_id));
        record.connected = false;
        record.mark_seen();
    }

    pub fn record_failure(&mut self, peer_id: PeerId) {
        let record = self
            .peers
            .entry(peer_id)
            .or_insert_with(|| PeerRecord::new(peer_id));
        record.failures = record.failures.saturating_add(1);
        record.mark_seen();
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
        self.peers.values().filter(|peer| peer.connected).count()
    }

    #[must_use]
    pub fn discovered_count(&self) -> usize {
        self.peers.values().filter(|peer| !peer.connected).count()
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
        book.record_connected(peer, None);

        let peers = book.peers();
        assert_eq!(peers.len(), 1);
        assert!(peers[0].connected);
        assert!(peers[0].has_source(PeerSource::Bootstrap));
        assert!(peers[0].has_source(PeerSource::PeerCache));
        assert!(peers[0].has_source(PeerSource::DhtProvider));
        assert_eq!(peers[0].namespace.as_deref(), Some("p2p-net/1/app/tag"));
    }
}
