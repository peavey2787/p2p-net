//! Reservation addresses this node requested, confirmed when the relay accepts.
//!
//! libp2p reports a relayed listen address built from the relay's own
//! advertised (usually public) addresses. A relay reached on a LAN endpoint is
//! then missing from the dial binding, although same-network peers (including
//! browsers that can only dial webrtc-direct/WebSocket) can use it.

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

use super::{relay_peer_id, RelayState};

impl RelayState {
    /// Remember a `/p2p-circuit` reservation address passed to `listen_on`.
    pub(crate) fn note_requested_reservation(&mut self, reservation_addr: &Multiaddr) {
        if let Some(relay) = relay_peer_id(reservation_addr) {
            self.requested_relay_listen_addrs
                .entry(relay)
                .or_default()
                .insert(reservation_addr.clone());
        }
    }

    /// Requested routes through `relay`, completed with the local peer id,
    /// once the relay has accepted the reservation.
    pub(crate) fn requested_routes(&self, relay: &PeerId, local: PeerId) -> Vec<Multiaddr> {
        self.requested_relay_listen_addrs
            .get(relay)
            .into_iter()
            .flatten()
            .map(|addr| addr.clone().with(Protocol::P2p(local)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_reservation_yields_requested_lan_route() {
        let relay = PeerId::random();
        let local = PeerId::random();
        let requested: Multiaddr =
            format!("/ip4/192.168.0.5/udp/9/webrtc-direct/p2p/{relay}/p2p-circuit")
                .parse()
                .unwrap();
        let mut state = RelayState::default();
        state.note_requested_reservation(&requested);
        assert_eq!(
            state.requested_routes(&relay, local),
            vec![requested.with(Protocol::P2p(local))]
        );
        assert!(state.requested_routes(&PeerId::random(), local).is_empty());
    }
}
