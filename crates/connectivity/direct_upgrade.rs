//! Browser relay-assisted direct upgrade (libp2p WebRTC private-to-private).
//!
//! Two browsers that can only reach each other through a Circuit Relay use the
//! relayed connection to exchange WebRTC signaling, then open a direct
//! `RtcPeerConnection`. When that direct connection is up, application
//! traffic moves to it: the relayed connections to that peer are closed. When
//! the upgrade fails, the relayed connection simply stays as the path.
//! Native peers keep using DCUtR; this module never applies to them.
//!
//! Lifecycle tags written to the node's pulses (concise; never per packet):
//! `DIRECT_DIAL`, `RELAY_CONNECT`, `WEBRTC_SIGNALING`,
//! `DIRECT_UPGRADE_SUCCESS`, `DIRECT_UPGRADE_FAILED`, `RELAY_FALLBACK`,
//! `PATH_MIGRATED_TO_DIRECT`, plus one `ICE_CHECK` per upgrade whose
//! connectivity check finished (`outcome=connected|failed|timeout`).

use std::collections::HashMap;

use libp2p::multiaddr::Protocol;
use libp2p::swarm::ConnectionId;
use libp2p::{Multiaddr, PeerId};

/// Path and counters of relay-assisted direct upgrades.
#[derive(Debug, Default, Clone)]
pub struct DirectUpgradeState {
    relayed: HashMap<PeerId, Vec<ConnectionId>>,
    pub upgrades_succeeded: u64,
    pub upgrades_failed: u64,
    pub paths_migrated: u64,
}

impl DirectUpgradeState {
    pub(crate) fn relayed_established(&mut self, peer: PeerId, connection: ConnectionId) {
        let connections = self.relayed.entry(peer).or_default();
        if !connections.contains(&connection) {
            connections.push(connection);
        }
    }

    pub(crate) fn connection_closed(&mut self, peer: PeerId, connection: ConnectionId) {
        if let Some(connections) = self.relayed.get_mut(&peer) {
            connections.retain(|existing| *existing != connection);
            if connections.is_empty() {
                self.relayed.remove(&peer);
            }
        }
    }

    /// Relayed connections to `peer` that a new direct path replaces.
    pub(crate) fn take_relayed(&mut self, peer: &PeerId) -> Vec<ConnectionId> {
        self.relayed.remove(peer).unwrap_or_default()
    }

    #[must_use]
    pub fn has_relayed(&self, peer: &PeerId) -> bool {
        self.relayed.contains_key(peer)
    }

    #[must_use]
    pub fn snapshot(&self) -> DirectUpgradeSnapshot {
        DirectUpgradeSnapshot {
            upgrades_succeeded: self.upgrades_succeeded,
            upgrades_failed: self.upgrades_failed,
            paths_migrated: self.paths_migrated,
            relayed_peers: self.relayed.len(),
        }
    }
}

/// Relay-assisted direct upgrade counters for snapshots and metrics.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirectUpgradeSnapshot {
    pub upgrades_succeeded: u64,
    pub upgrades_failed: u64,
    /// Peers whose traffic moved from relayed connections to a direct one.
    pub paths_migrated: u64,
    /// Peers currently reached only through relayed connections.
    pub relayed_peers: usize,
}

/// A browser-to-browser direct WebRTC connection address (`/webrtc`, not
/// `/webrtc-direct`, not through a circuit).
#[must_use]
pub fn is_direct_browser_webrtc(addr: &Multiaddr) -> bool {
    let mut webrtc = false;
    for protocol in addr.iter() {
        match protocol {
            Protocol::P2pCircuit | Protocol::WebRTCDirect => return false,
            Protocol::WebRTC => webrtc = true,
            _ => {}
        }
    }
    webrtc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_webrtc_outside_a_circuit_is_a_direct_browser_path() {
        let peer = PeerId::random();
        let direct: Multiaddr = format!("/webrtc/p2p/{peer}").parse().unwrap();
        let signaling: Multiaddr = format!("/ip4/1.2.3.4/tcp/1/p2p/{peer}/p2p-circuit/webrtc")
            .parse()
            .unwrap();
        let server: Multiaddr = "/ip4/1.2.3.4/udp/1/webrtc-direct".parse().unwrap();
        assert!(is_direct_browser_webrtc(&direct));
        assert!(!is_direct_browser_webrtc(&signaling));
        assert!(!is_direct_browser_webrtc(&server));
    }

    #[test]
    fn migration_takes_every_relayed_connection_to_the_peer_once() {
        let mut state = DirectUpgradeState::default();
        let (peer, other) = (PeerId::random(), PeerId::random());
        let (a, b, c) = (
            ConnectionId::new_unchecked(1),
            ConnectionId::new_unchecked(2),
            ConnectionId::new_unchecked(3),
        );
        state.relayed_established(peer, a);
        state.relayed_established(peer, b);
        state.relayed_established(other, c);
        assert_eq!(state.take_relayed(&peer), vec![a, b]);
        assert!(state.take_relayed(&peer).is_empty());
        assert!(state.has_relayed(&other));
        state.connection_closed(other, c);
        assert!(!state.has_relayed(&other));
    }
}
