use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

/// Convert a configured relay peer address into the relayed listen address
/// that makes rust-libp2p request a Circuit Relay v2 reservation.
///
/// Example:
/// `/ip4/127.0.0.1/tcp/4001/p2p/<relay>` ->
/// `/ip4/127.0.0.1/tcp/4001/p2p/<relay>/p2p-circuit`
pub fn relay_reservation_addr(relay_addr: &Multiaddr) -> Option<Multiaddr> {
    if !has_p2p_peer_id(relay_addr) || is_p2p_circuit_addr(relay_addr) {
        return None;
    }

    Some(relay_addr.clone().with(Protocol::P2pCircuit))
}

/// Return true for any address that contains `/p2p-circuit`.
pub fn is_p2p_circuit_addr(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
}

/// Extract the relay peer ID from a relay or relayed address.
///
/// For `/ip4/.../p2p/<relay>/p2p-circuit/p2p/<target>`, this returns `<relay>`.
pub fn relay_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    for protocol in addr.iter() {
        match protocol {
            Protocol::P2p(peer) => return Some(peer),
            Protocol::P2pCircuit => return None,
            _ => {}
        }
    }
    None
}

fn has_p2p_peer_id(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::P2p(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_relay_reservation_address() {
        let relay_peer = PeerId::random();
        let relay_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay_peer}")
            .parse()
            .unwrap();
        let reservation = relay_reservation_addr(&relay_addr).unwrap();
        assert_eq!(
            reservation.to_string(),
            format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay_peer}/p2p-circuit")
        );
        assert_eq!(relay_peer_id(&reservation), Some(relay_peer));
        assert!(is_p2p_circuit_addr(&reservation));
    }

    #[test]
    fn reservation_address_rejects_non_relay_addr() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        assert!(relay_reservation_addr(&addr).is_none());
    }
}
