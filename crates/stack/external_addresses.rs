use std::collections::{HashSet, VecDeque};
use std::convert::Infallible;
use std::task::{Context, Poll};

use libp2p::core::{transport::PortUse, Endpoint};
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, Swarm, THandler,
    THandlerInEvent, THandlerOutEvent, ToSwarm,
};
use libp2p::{Multiaddr, PeerId};

const MAX_EXTERNAL_ADDRESS_CACHE: usize = 32;

/// Bridges application-confirmed public addresses into libp2p behaviours that
/// consume external-address candidates, including DCUtR.
pub struct ExternalAddressCandidates {
    pending: VecDeque<ExternalAddressAction>,
    candidate_seen: HashSet<Multiaddr>,
    candidate_order: VecDeque<Multiaddr>,
    confirmed_seen: HashSet<Multiaddr>,
    confirmed_order: VecDeque<Multiaddr>,
}

enum ExternalAddressAction {
    Candidate(Multiaddr),
    Confirm(Multiaddr),
    Expire(Multiaddr),
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
            candidate_order: VecDeque::new(),
            confirmed_seen: HashSet::new(),
            confirmed_order: VecDeque::new(),
        }
    }

    pub fn add_candidate(&mut self, address: Multiaddr) {
        if !supports_dcutr_port_reuse(&address) {
            return;
        }
        if remember_bounded(
            &mut self.candidate_seen,
            &mut self.candidate_order,
            address.clone(),
            MAX_EXTERNAL_ADDRESS_CACHE,
        )
        .is_some()
        {
            self.pending
                .push_back(ExternalAddressAction::Candidate(address));
        }
    }

    pub fn add_confirmed(&mut self, address: Multiaddr) {
        self.add_candidate(address.clone());
        if let Some(evicted) = remember_bounded(
            &mut self.confirmed_seen,
            &mut self.confirmed_order,
            address.clone(),
            MAX_EXTERNAL_ADDRESS_CACHE,
        ) {
            if evicted != address {
                self.pending
                    .push_back(ExternalAddressAction::Expire(evicted));
            }
            self.pending
                .push_back(ExternalAddressAction::Confirm(address));
        }
    }
}

fn remember_bounded(
    seen: &mut HashSet<Multiaddr>,
    order: &mut VecDeque<Multiaddr>,
    address: Multiaddr,
    max_entries: usize,
) -> Option<Multiaddr> {
    if !seen.insert(address.clone()) {
        return None;
    }
    order.push_back(address.clone());
    while seen.len() > max_entries {
        let Some(evicted) = order.pop_front() else {
            break;
        };
        if seen.remove(&evicted) {
            return Some(evicted);
        }
    }
    Some(address)
}

fn supports_dcutr_port_reuse(address: &Multiaddr) -> bool {
    #[cfg(target_os = "windows")]
    {
        // rust-libp2p TCP uses listener-port reuse for DCUtR simultaneous-open.
        // Windows does not provide the required SO_REUSEPORT behavior, so
        // offering TCP/WS candidates only produces AddrInUse failures and can
        // evict viable QUIC candidates from DCUtR's bounded candidate cache.
        return !address
            .iter()
            .any(|protocol| matches!(protocol, libp2p::multiaddr::Protocol::Tcp(_)));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = address;
        true
    }
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
            Some(ExternalAddressAction::Expire(address)) => {
                Poll::Ready(ToSwarm::ExternalAddrExpired(address))
            }
            None => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmed_addresses_stay_advertised_when_not_dcutr_candidates() {
        let tcp: Multiaddr = "/ip4/203.0.113.1/tcp/4001".parse().unwrap();
        let mut behaviour = ExternalAddressCandidates::new();

        behaviour.add_confirmed(tcp.clone());

        assert!(behaviour.confirmed_seen.contains(&tcp));
        if cfg!(target_os = "windows") {
            assert!(!behaviour.candidate_seen.contains(&tcp));
        } else {
            assert!(behaviour.candidate_seen.contains(&tcp));
        }
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
        for suffix in 1..=MAX_EXTERNAL_ADDRESS_CACHE + 5 {
            let addr: Multiaddr = format!("/ip4/203.0.113.{suffix}/udp/4001/quic-v1")
                .parse()
                .unwrap();
            behaviour.add_confirmed(addr);
        }

        assert_eq!(behaviour.candidate_seen.len(), MAX_EXTERNAL_ADDRESS_CACHE);
        assert_eq!(behaviour.confirmed_seen.len(), MAX_EXTERNAL_ADDRESS_CACHE);
    }
}
