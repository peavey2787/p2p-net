use std::collections::{HashSet, VecDeque};
use std::convert::Infallible;
use std::task::{Context, Poll};

use libp2p::core::{transport::PortUse, Endpoint};
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, Swarm, THandler,
    THandlerInEvent, THandlerOutEvent, ToSwarm,
};
use libp2p::{Multiaddr, PeerId};

const MAX_EXTERNAL_DIRECT_ADDRESSES: usize = 16;
const MAX_EXTERNAL_RELAY_ADDRESSES: usize = 16;

/// Bridges application-confirmed public addresses into libp2p behaviours that
/// consume external-address candidates, including DCUtR.
pub struct ExternalAddressCandidates {
    pending: VecDeque<ExternalAddressAction>,
    candidate_seen: HashSet<Multiaddr>,
    confirmed_seen: HashSet<Multiaddr>,
}

enum ExternalAddressAction {
    Candidate(Multiaddr),
    Confirm(Multiaddr),
}

impl Default for ExternalAddressCandidates {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalAddressCandidates {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            candidate_seen: HashSet::new(),
            confirmed_seen: HashSet::new(),
        }
    }

    pub fn add_candidate(&mut self, address: Multiaddr) {
        if remember_bounded(&mut self.candidate_seen, address.clone()) {
            self.pending
                .push_back(ExternalAddressAction::Candidate(address));
        }
    }

    pub fn add_confirmed(&mut self, address: Multiaddr) {
        self.add_candidate(address.clone());
        if remember_bounded(&mut self.confirmed_seen, address.clone()) {
            self.pending
                .push_back(ExternalAddressAction::Confirm(address));
        }
    }
}

fn remember_bounded(seen: &mut HashSet<Multiaddr>, address: Multiaddr) -> bool {
    if seen.contains(&address) {
        return false;
    }
    let relayed = is_relayed(&address);
    let category_count = seen
        .iter()
        .filter(|known| is_relayed(known) == relayed)
        .count();
    let category_limit = if relayed {
        MAX_EXTERNAL_RELAY_ADDRESSES
    } else {
        MAX_EXTERNAL_DIRECT_ADDRESSES
    };
    if category_count >= category_limit {
        // Endpoint-dependent NAT observations can produce an unbounded stream
        // of one-off ports. Rotating the cache here turns every observation
        // into candidate/expire/confirm swarm events forever. Keep the first
        // bounded working set for this runtime instead.
        return false;
    }
    seen.insert(address)
}

fn is_relayed(address: &Multiaddr) -> bool {
    address
        .iter()
        .any(|protocol| matches!(protocol, libp2p::multiaddr::Protocol::P2pCircuit))
}

pub fn add_external_address_candidate(swarm: &mut Swarm<super::MeshBehaviour>, address: Multiaddr) {
    swarm
        .behaviour_mut()
        .external_address_candidates
        .add_confirmed(address);
}

pub fn add_hole_punch_candidate(swarm: &mut Swarm<super::MeshBehaviour>, address: Multiaddr) {
    swarm
        .behaviour_mut()
        .external_address_candidates
        .add_candidate(address);
}

impl NetworkBehaviour for ExternalAddressCandidates {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = Infallible;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, _: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        libp2p::core::util::unreachable(event)
    }

    fn poll(&mut self, _: &mut Context<'_>) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        match self.pending.pop_front() {
            Some(ExternalAddressAction::Candidate(address)) => {
                Poll::Ready(ToSwarm::NewExternalAddrCandidate(address))
            }
            Some(ExternalAddressAction::Confirm(address)) => {
                Poll::Ready(ToSwarm::ExternalAddrConfirmed(address))
            }
            None => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_addresses_are_candidates_on_every_native_platform() {
        let tcp: Multiaddr = "/ip4/203.0.113.1/tcp/4001".parse().unwrap();
        let mut behaviour = ExternalAddressCandidates::new();

        behaviour.add_confirmed(tcp.clone());

        assert!(behaviour.confirmed_seen.contains(&tcp));
        assert!(behaviour.candidate_seen.contains(&tcp));
    }

    #[test]
    fn quic_addresses_remain_dcutr_candidates() {
        let quic: Multiaddr = "/ip4/203.0.113.1/udp/4001/quic-v1".parse().unwrap();
        let mut behaviour = ExternalAddressCandidates::new();

        behaviour.add_confirmed(quic.clone());

        assert!(behaviour.candidate_seen.contains(&quic));
        assert!(behaviour.confirmed_seen.contains(&quic));
    }

    #[test]
    fn observed_external_address_sets_are_bounded() {
        let mut behaviour = ExternalAddressCandidates::new();
        for suffix in 1..=MAX_EXTERNAL_DIRECT_ADDRESSES + 5 {
            let addr: Multiaddr = format!("/ip4/203.0.113.{suffix}/udp/4001/quic-v1")
                .parse()
                .unwrap();
            behaviour.add_confirmed(addr);
        }

        assert_eq!(
            behaviour.candidate_seen.len(),
            MAX_EXTERNAL_DIRECT_ADDRESSES
        );
        assert_eq!(
            behaviour.confirmed_seen.len(),
            MAX_EXTERNAL_DIRECT_ADDRESSES
        );
        assert_eq!(behaviour.pending.len(), MAX_EXTERNAL_DIRECT_ADDRESSES * 2);
    }
}
