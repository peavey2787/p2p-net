use std::collections::HashSet;
use std::task::{Context, Poll};

use libp2p::core::{transport::PortUse, Endpoint};
use libp2p::multiaddr::Protocol;
use libp2p::swarm::{
    ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{dcutr, Multiaddr, PeerId};

use crate::connectivity::addr::is_public_direct_addr;

const MAX_PUBLIC_QUIC_CANDIDATES: usize = 8;

/// Restricts DCUtR to transports that support listener-port reuse on every
/// native platform shipped by p2p-net.
///
/// Identify can emit many observed addresses directly into every behaviour.
/// Passing all of them to rust-libp2p DCUtR makes Windows attempt unsupported
/// TCP simultaneous-open and can evict useful LAN/QUIC candidates from DCUtR's
/// bounded cache.
pub struct DcutrBehaviour {
    inner: dcutr::Behaviour,
    public_quic_candidates: HashSet<Multiaddr>,
}

impl DcutrBehaviour {
    pub fn new(local_peer: PeerId) -> Self {
        Self {
            inner: dcutr::Behaviour::new(local_peer),
            public_quic_candidates: HashSet::new(),
        }
    }

    fn accept_candidate(&mut self, addr: &Multiaddr) -> bool {
        if !is_quic_candidate(addr) {
            return false;
        }
        if !is_public_direct_addr(addr) || self.public_quic_candidates.contains(addr) {
            return true;
        }
        if self.public_quic_candidates.len() >= MAX_PUBLIC_QUIC_CANDIDATES {
            return false;
        }
        self.public_quic_candidates.insert(addr.clone());
        true
    }
}

fn is_quic_candidate(addr: &Multiaddr) -> bool {
    let mut has_udp = false;
    let mut has_quic = false;
    for protocol in addr.iter() {
        match protocol {
            Protocol::Udp(_) => has_udp = true,
            Protocol::QuicV1 => has_quic = true,
            Protocol::Tcp(_) | Protocol::Ws(_) | Protocol::Wss(_) | Protocol::P2pCircuit => {
                return false
            }
            Protocol::Ip4(ip) if ip.is_unspecified() => return false,
            Protocol::Ip6(ip) if ip.is_unspecified() => return false,
            _ => {}
        }
    }
    has_udp && has_quic
}

impl NetworkBehaviour for DcutrBehaviour {
    type ConnectionHandler = THandler<dcutr::Behaviour>;
    type ToSwarm = dcutr::Event;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.inner.handle_established_inbound_connection(
            connection_id,
            peer,
            local_addr,
            remote_addr,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.inner.handle_established_outbound_connection(
            connection_id,
            peer,
            addr,
            role_override,
            port_use,
        )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::NewExternalAddrCandidate(candidate) = &event {
            if !self.accept_candidate(candidate.addr) {
                return;
            }
        }
        self.inner.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.inner
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        self.inner.poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dcutr_candidates_are_quic_only() {
        let quic: Multiaddr = "/ip4/192.168.1.2/udp/4001/quic-v1".parse().unwrap();
        let tcp: Multiaddr = "/ip4/192.168.1.2/tcp/4001".parse().unwrap();

        assert!(is_quic_candidate(&quic));
        assert!(!is_quic_candidate(&tcp));
    }

    #[test]
    fn public_candidate_count_is_bounded() {
        let mut behaviour = DcutrBehaviour::new(PeerId::random());
        for suffix in 1..=MAX_PUBLIC_QUIC_CANDIDATES {
            let addr = format!("/ip4/8.8.8.{suffix}/udp/4001/quic-v1")
                .parse()
                .unwrap();
            assert!(behaviour.accept_candidate(&addr));
        }
        let overflow: Multiaddr = "/ip4/9.9.9.9/udp/4001/quic-v1".parse().unwrap();
        let lan: Multiaddr = "/ip4/192.168.1.2/udp/4001/quic-v1".parse().unwrap();

        assert!(!behaviour.accept_candidate(&overflow));
        assert!(behaviour.accept_candidate(&lan));
    }
}
