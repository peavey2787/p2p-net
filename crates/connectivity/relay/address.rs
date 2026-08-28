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

/// Turn a relay reservation/listen address into a dialable route for `target`.
/// The relay peer ID before `/p2p-circuit` is preserved and the target peer ID
/// is appended after the circuit component. Existing routes for another target
/// are rejected.
pub fn relay_dial_addr_for_peer(addr: &Multiaddr, target: PeerId) -> Option<Multiaddr> {
    if !is_p2p_circuit_addr(addr) {
        return None;
    }
    let mut seen_circuit = false;
    let mut target_after_circuit = None;
    for protocol in addr.iter() {
        match protocol {
            Protocol::P2pCircuit => seen_circuit = true,
            Protocol::P2p(peer) if seen_circuit => target_after_circuit = Some(peer),
            _ => {}
        }
    }
    match target_after_circuit {
        Some(peer) if peer == target => Some(addr.clone()),
        Some(_) => None,
        None => Some(addr.clone().with(Protocol::P2p(target))),
    }
}

fn has_p2p_peer_id(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::P2p(_)))
}

#[cfg(test)]
mod tests;
