use std::collections::{HashMap, HashSet, VecDeque};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use either::Either;
use libp2p::core::{transport::PortUse, Endpoint};
use libp2p::multiaddr::Protocol;
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{dcutr, Multiaddr, PeerId};

use crate::connectivity::addr::is_public_direct_addr;

const MAX_PUBLIC_QUIC_CANDIDATES: usize = 8;
const MAX_ALLOWED_DCUTR_PEERS: usize = 1024;

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
    retry_interval: Duration,
    max_attempts_per_peer: u32,
    allowed_peers: HashSet<PeerId>,
    allowed_peer_order: VecDeque<PeerId>,
    attempts_by_peer: HashMap<PeerId, u32>,
    last_attempt_by_peer: HashMap<PeerId, Instant>,
}

impl DcutrBehaviour {
    pub fn new(local_peer: PeerId, retry_interval_secs: u64, max_attempts_per_peer: u32) -> Self {
        Self {
            inner: dcutr::Behaviour::new(local_peer),
            public_quic_candidates: HashSet::new(),
            retry_interval: Duration::from_secs(retry_interval_secs.max(1)),
            max_attempts_per_peer: max_attempts_per_peer.max(1),
            allowed_peers: HashSet::new(),
            allowed_peer_order: VecDeque::new(),
            attempts_by_peer: HashMap::new(),
            last_attempt_by_peer: HashMap::new(),
        }
    }

    pub fn allow_peer(&mut self, peer: PeerId) {
        if !self.allowed_peers.insert(peer) {
            return;
        }
        self.allowed_peer_order.push_back(peer);
        while self.allowed_peers.len() > MAX_ALLOWED_DCUTR_PEERS {
            let Some(evicted) = self.allowed_peer_order.pop_front() else {
                break;
            };
            if self.allowed_peers.remove(&evicted) {
                self.attempts_by_peer.remove(&evicted);
                self.last_attempt_by_peer.remove(&evicted);
            }
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

    fn allow_relayed_upgrade(&mut self, peer: PeerId, require_allowlist: bool) -> bool {
        if require_allowlist && !self.allowed_peers.contains(&peer) {
            return false;
        }
        if !self.allowed_peers.contains(&peer) {
            self.allow_peer(peer);
        }
        let attempts = self.attempts_by_peer.entry(peer).or_default();
        if *attempts >= self.max_attempts_per_peer {
            return false;
        }

        let now = Instant::now();
        if self
            .last_attempt_by_peer
            .get(&peer)
            .is_some_and(|last| now.duration_since(*last) < self.retry_interval)
        {
            return false;
        }

        *attempts = attempts.saturating_add(1);
        self.last_attempt_by_peer.insert(peer, now);
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
            Protocol::Ip4(ip) if ip.is_loopback() || ip.is_link_local() => return false,
            Protocol::Ip6(ip) if ip.is_loopback() => return false,
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
        if local_addr
            .iter()
            .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
            && !self.allow_relayed_upgrade(peer, true)
        {
            return Ok(Either::Right(dummy::ConnectionHandler));
        }
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
        if addr
            .iter()
            .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
            && !self.allow_relayed_upgrade(peer, true)
        {
            return Ok(Either::Right(dummy::ConnectionHandler));
        }
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
        let loopback: Multiaddr = "/ip4/127.0.0.1/udp/4001/quic-v1".parse().unwrap();

        assert!(is_quic_candidate(&quic));
        assert!(!is_quic_candidate(&tcp));
        assert!(!is_quic_candidate(&loopback));
    }

    #[test]
    fn public_candidate_count_is_bounded() {
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3);
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

    #[test]
    fn relayed_upgrade_policy_enforces_attempt_budget_and_cooldown() {
        let peer = PeerId::random();
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 2);
        behaviour.allow_peer(peer);

        assert!(behaviour.allow_relayed_upgrade(peer, true));
        assert!(!behaviour.allow_relayed_upgrade(peer, true));

        let last = behaviour
            .last_attempt_by_peer
            .get_mut(&peer)
            .expect("last attempt recorded");
        *last = last
            .checked_sub(Duration::from_secs(61))
            .expect("instant subtracts");
        assert!(behaviour.allow_relayed_upgrade(peer, true));

        let last = behaviour
            .last_attempt_by_peer
            .get_mut(&peer)
            .expect("last attempt recorded");
        *last = last
            .checked_sub(Duration::from_secs(61))
            .expect("instant subtracts");
        assert!(!behaviour.allow_relayed_upgrade(peer, true));
    }

    #[test]
    fn relayed_upgrade_policy_rejects_unmarked_peers() {
        let peer = PeerId::random();
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 2);

        assert!(!behaviour.allow_relayed_upgrade(peer, true));
        behaviour.allow_peer(peer);
        assert!(behaviour.allow_relayed_upgrade(peer, true));
    }

    #[test]
    fn dcutr_allowlist_is_bounded() {
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 2);
        for _ in 0..MAX_ALLOWED_DCUTR_PEERS + 5 {
            behaviour.allow_peer(PeerId::random());
        }

        assert_eq!(behaviour.allowed_peers.len(), MAX_ALLOWED_DCUTR_PEERS);
    }
}
