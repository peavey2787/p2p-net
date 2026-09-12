use std::collections::{HashMap, HashSet, VecDeque};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use either::Either;
use libp2p::core::{transport::PortUse, ConnectedPoint, Endpoint};
use libp2p::multiaddr::Protocol;
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent, ConnectionId, FromSwarm,
    NetworkBehaviour, NotifyHandler, SubstreamProtocol, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{dcutr, Multiaddr, PeerId};

use crate::connectivity::addr::is_public_direct_addr;

const MAX_ALLOWED_DCUTR_PEERS: usize = 1024;

/// Offers bounded TCP and QUIC candidates to verified application peers.
/// The upstream behaviour maintains a bounded, refreshing candidate cache.
/// LAN candidates follow the node's opt-in; fresh WAN observations must not
/// be rejected just because earlier NAT mappings filled a lifetime quota.
pub struct DcutrBehaviour {
    inner: dcutr::Behaviour,
    has_candidate: bool,
    retry_interval: Duration,
    max_attempts_per_peer: u32,
    allowed_peers: HashSet<PeerId>,
    allowed_peer_order: VecDeque<PeerId>,
    attempts_by_peer: HashMap<PeerId, u32>,
    last_attempt_by_peer: HashMap<PeerId, Instant>,
    allow_lan_candidates: bool,
    deferred: HashMap<ConnectionId, (PeerId, ConnectedPoint)>,
    pending_handlers: VecDeque<(PeerId, ConnectionId, THandler<dcutr::Behaviour>)>,
}

impl DcutrBehaviour {
    pub fn new(local_peer: PeerId, retry_interval_secs: u64, max_attempts_per_peer: u32) -> Self {
        Self {
            inner: dcutr::Behaviour::new(local_peer),
            has_candidate: false,
            retry_interval: Duration::from_secs(retry_interval_secs.max(1)),
            max_attempts_per_peer: max_attempts_per_peer.max(1),
            allowed_peers: HashSet::new(),
            allowed_peer_order: VecDeque::new(),
            attempts_by_peer: HashMap::new(),
            last_attempt_by_peer: HashMap::new(),
            allow_lan_candidates: true,
            deferred: HashMap::new(),
            pending_handlers: VecDeque::new(),
        }
    }

    /// Apply the node's LAN opt-in to the addresses offered for hole punching.
    pub fn with_lan_candidates(mut self, enabled: bool) -> Self {
        self.allow_lan_candidates = enabled;
        self
    }

    pub fn allow_peer(&mut self, peer: PeerId) {
        if self.allowed_peers.insert(peer) {
            self.allowed_peer_order.push_back(peer);
        }
        while self.allowed_peers.len() > MAX_ALLOWED_DCUTR_PEERS {
            let Some(evicted) = self.allowed_peer_order.pop_front() else {
                break;
            };
            if self.allowed_peers.remove(&evicted) {
                self.attempts_by_peer.remove(&evicted);
                self.last_attempt_by_peer.remove(&evicted);
            }
        }
        self.activate_deferred(peer);
    }

    fn activate_deferred(&mut self, peer: PeerId) {
        if !self.has_candidate || !self.allowed_peers.contains(&peer) {
            return;
        }
        // Identify may verify an inbound circuit only after its handler was
        // created. Enable DCUtR on that same circuit, without dropping the
        // application connection or creating a competing relay circuit.
        let connections = self
            .deferred
            .iter()
            .filter_map(|(id, (remote, _))| (*remote == peer).then_some(*id))
            .collect::<Vec<_>>();
        for id in connections {
            let Some((_, endpoint)) = self.deferred.remove(&id) else {
                continue;
            };
            if endpoint.is_listener() && !self.allow_relayed_upgrade(peer, true) {
                continue;
            }
            let handler = match endpoint {
                ConnectedPoint::Listener {
                    local_addr,
                    send_back_addr,
                } => self.inner.handle_established_inbound_connection(
                    id,
                    peer,
                    &local_addr,
                    &send_back_addr,
                ),
                ConnectedPoint::Dialer {
                    address,
                    role_override,
                    port_use,
                } => self.inner.handle_established_outbound_connection(
                    id,
                    peer,
                    &address,
                    role_override,
                    port_use,
                ),
            };
            if let Ok(handler) = handler {
                self.pending_handlers.push_back((peer, id, handler));
            }
        }
    }

    fn accept_candidate(&mut self, addr: &Multiaddr) -> bool {
        if !is_dcutr_candidate(addr) {
            return false;
        }
        if !self.allow_lan_candidates && !is_public_direct_addr(addr) {
            return false;
        }
        // libp2p-dcutr 0.14.1 already uses a 20-entry LRU. Our former
        // first-eight-per-transport gate prevented that cache from refreshing
        // after NAT rebinding. Forward observations without retaining another
        // address history or emitting any extra confirmation/expiry events.
        self.has_candidate = true;
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

fn is_dcutr_candidate(addr: &Multiaddr) -> bool {
    let mut protocols = addr.iter();
    match protocols.next() {
        Some(Protocol::Ip4(ip))
            if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() => {}
        Some(Protocol::Ip6(ip))
            if !ip.is_loopback() && !ip.is_unspecified() && !ip.is_unicast_link_local() => {}
        _ => return false,
    }
    match protocols.next() {
        Some(Protocol::Tcp(port)) if port != 0 => {}
        Some(Protocol::Udp(port)) if port != 0 => {
            if protocols.next() != Some(Protocol::QuicV1) {
                return false;
            }
        }
        _ => return false,
    }
    // WS, WebTransport and WebRTC have additional handshake requirements and
    // are not bare TCP/QUIC DCUtR candidates. A terminal identity is permitted.
    match protocols.next() {
        None => true,
        Some(Protocol::P2p(_)) => protocols.next().is_none(),
        _ => false,
    }
}

impl NetworkBehaviour for DcutrBehaviour {
    type ConnectionHandler = VerifiedDcutrHandler;
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
            && (!self.has_candidate || !self.allow_relayed_upgrade(peer, true))
        {
            if !self.has_candidate || !self.allowed_peers.contains(&peer) {
                self.deferred.insert(
                    connection_id,
                    (
                        peer,
                        ConnectedPoint::Listener {
                            local_addr: local_addr.clone(),
                            send_back_addr: remote_addr.clone(),
                        },
                    ),
                );
            }
            return Ok(VerifiedDcutrHandler(Either::Right(
                dummy::ConnectionHandler,
            )));
        }
        self.inner
            .handle_established_inbound_connection(connection_id, peer, local_addr, remote_addr)
            .map(VerifiedDcutrHandler)
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
            && (!self.has_candidate || !self.allowed_peers.contains(&peer))
        {
            self.deferred.insert(
                connection_id,
                (
                    peer,
                    ConnectedPoint::Dialer {
                        address: addr.clone(),
                        role_override,
                        port_use,
                    },
                ),
            );
            return Ok(VerifiedDcutrHandler(Either::Right(
                dummy::ConnectionHandler,
            )));
        }
        // The dialer of a relay circuit responds to DCUtR; the circuit's
        // listener initiates it. Local initiation cooldowns must not remove a
        // verified peer's responder protocol. Otherwise asymmetric circuit
        // replacement turns a valid remote attempt into Unsupported.
        self.inner
            .handle_established_outbound_connection(
                connection_id,
                peer,
                addr,
                role_override,
                port_use,
            )
            .map(VerifiedDcutrHandler)
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::ConnectionClosed(closed) = &event {
            self.deferred.remove(&closed.connection_id);
            self.pending_handlers
                .retain(|(_, id, _)| *id != closed.connection_id);
        }
        let candidate = match &event {
            FromSwarm::NewExternalAddrCandidate(candidate) => Some(candidate.addr),
            FromSwarm::ExternalAddrConfirmed(confirmed) => Some(confirmed.addr),
            _ => None,
        };
        if let Some(addr) = candidate {
            if !self.accept_candidate(addr) {
                return;
            }
            // Swarm suppresses NewExternalAddrCandidate for already-confirmed
            // addresses, while upstream DCUtR only consumes candidate events.
            // Feed either source through the same bounded, WAN-aware policy.
            self.inner
                .on_swarm_event(FromSwarm::NewExternalAddrCandidate(
                    libp2p::swarm::behaviour::NewExternalAddrCandidate { addr },
                ));
            let peers = self
                .deferred
                .values()
                .map(|(peer, _)| *peer)
                .collect::<HashSet<_>>();
            for peer in peers {
                self.activate_deferred(peer);
            }
            return;
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
        if let Some((peer_id, connection_id, handler)) = self.pending_handlers.pop_front() {
            return Poll::Ready(ToSwarm::NotifyHandler {
                peer_id,
                handler: NotifyHandler::One(connection_id),
                event: DcutrHandlerCommand::Enable(Box::new(handler)),
            });
        }
        self.inner
            .poll(cx)
            .map(|event| event.map_in(DcutrHandlerCommand::Protocol))
    }
}

/// A disabled circuit handler can be activated once application verification
/// succeeds. The relay circuit and all other application streams stay intact.
pub struct VerifiedDcutrHandler(THandler<dcutr::Behaviour>);

pub enum DcutrHandlerCommand {
    Enable(Box<THandler<dcutr::Behaviour>>),
    Protocol(THandlerInEvent<dcutr::Behaviour>),
}

impl std::fmt::Debug for DcutrHandlerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Enable(_) => f.write_str("EnableVerifiedDcutr"),
            Self::Protocol(event) => event.fmt(f),
        }
    }
}

impl ConnectionHandler for VerifiedDcutrHandler {
    type FromBehaviour = DcutrHandlerCommand;
    type ToBehaviour = THandlerOutEvent<dcutr::Behaviour>;
    type InboundProtocol = <THandler<dcutr::Behaviour> as ConnectionHandler>::InboundProtocol;
    type OutboundProtocol = <THandler<dcutr::Behaviour> as ConnectionHandler>::OutboundProtocol;
    type InboundOpenInfo = <THandler<dcutr::Behaviour> as ConnectionHandler>::InboundOpenInfo;
    type OutboundOpenInfo = <THandler<dcutr::Behaviour> as ConnectionHandler>::OutboundOpenInfo;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        self.0.listen_protocol()
    }
    fn connection_keep_alive(&self) -> bool {
        self.0.connection_keep_alive()
    }
    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        match event {
            DcutrHandlerCommand::Enable(handler) => {
                if self.0.is_right() {
                    self.0 = *handler;
                }
            }
            DcutrHandlerCommand::Protocol(event) => self.0.on_behaviour_event(event),
        }
    }
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        self.0.poll(cx)
    }
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<Option<Self::ToBehaviour>> {
        self.0.poll_close(cx)
    }
    fn on_connection_event(
        &mut self,
        event: libp2p::swarm::handler::ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        self.0.on_connection_event(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_responder_remains_available_after_local_budget_is_spent() {
        let peer = PeerId::random();
        let relay = PeerId::random();
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 1);
        let candidate = "/ip4/8.8.4.4/tcp/4001".parse().unwrap();
        behaviour.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
            libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &candidate },
        ));
        behaviour.allow_peer(peer);
        assert!(behaviour.allow_relayed_upgrade(peer, true));
        assert!(!behaviour.allow_relayed_upgrade(peer, true));
        let addr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{peer}")
            .parse()
            .unwrap();
        let handler = behaviour
            .handle_established_outbound_connection(
                ConnectionId::new_unchecked(71),
                peer,
                &addr,
                Endpoint::Dialer,
                PortUse::Reuse,
            )
            .unwrap();
        assert!(
            handler.0.is_left(),
            "verified responders must still negotiate DCUtR"
        );
        assert_eq!(behaviour.attempts_by_peer[&peer], 1);
    }

    #[test]
    fn late_verified_responder_does_not_spend_local_initiation_budget() {
        let peer = PeerId::random();
        let relay = PeerId::random();
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 1);
        let addr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{peer}")
            .parse()
            .unwrap();
        let handler = behaviour
            .handle_established_outbound_connection(
                ConnectionId::new_unchecked(72),
                peer,
                &addr,
                Endpoint::Dialer,
                PortUse::Reuse,
            )
            .unwrap();
        assert!(handler.0.is_right());
        let candidate = "/ip4/8.8.4.4/tcp/4001".parse().unwrap();
        behaviour.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
            libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &candidate },
        ));
        assert!(
            behaviour.pending_handlers.is_empty(),
            "unverified responders stay disabled"
        );
        behaviour.allow_peer(peer);
        assert_eq!(behaviour.pending_handlers.len(), 1);
        assert!(behaviour.attempts_by_peer.is_empty());
    }

    #[test]
    fn late_verification_activates_existing_relay_handler_once() {
        let peer = PeerId::random();
        let relay = PeerId::random();
        let id = ConnectionId::new_unchecked(1);
        let addr = format!("/ip4/8.8.8.8/udp/4001/quic-v1/p2p/{relay}/p2p-circuit")
            .parse()
            .unwrap();
        let remote = format!("/p2p/{peer}").parse().unwrap();
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3);
        let candidate = "/ip4/8.8.4.4/tcp/4001".parse().unwrap();
        behaviour.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
            libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &candidate },
        ));
        let mut handler = behaviour
            .handle_established_inbound_connection(id, peer, &addr, &remote)
            .unwrap();
        assert!(handler.0.is_right());
        assert_eq!(behaviour.deferred.len(), 1);
        assert!(behaviour.attempts_by_peer.is_empty());

        behaviour.allow_peer(peer);
        assert!(behaviour.deferred.is_empty());
        assert_eq!(behaviour.attempts_by_peer[&peer], 1);
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let Poll::Ready(ToSwarm::NotifyHandler {
            peer_id,
            handler: NotifyHandler::One(connection),
            event,
        }) = behaviour.poll(&mut cx)
        else {
            panic!("verification must activate the existing circuit");
        };
        assert_eq!(peer_id, peer);
        assert_eq!(connection, id);
        handler.on_behaviour_event(event);
        assert!(handler.0.is_left());
        assert!(matches!(
            handler.poll(&mut cx),
            Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest { .. })
        ));

        behaviour.allow_peer(peer);
        assert!(behaviour.pending_handlers.is_empty());
        assert_eq!(behaviour.attempts_by_peer[&peer], 1);
    }

    #[test]
    fn verified_circuit_waits_for_a_public_candidate_without_spending_budget() {
        let peer = PeerId::random();
        let relay = PeerId::random();
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3).with_lan_candidates(false);
        behaviour.allow_peer(peer);
        let id = ConnectionId::new_unchecked(9);
        let addr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay}/p2p-circuit")
            .parse()
            .unwrap();
        let remote = format!("/p2p/{peer}").parse().unwrap();
        let handler = behaviour
            .handle_established_inbound_connection(id, peer, &addr, &remote)
            .unwrap();
        assert!(handler.0.is_right());
        assert!(behaviour.attempts_by_peer.is_empty());
        let private = "/ip4/192.168.0.53/tcp/4001".parse().unwrap();
        behaviour.on_swarm_event(FromSwarm::ExternalAddrConfirmed(
            libp2p::swarm::behaviour::ExternalAddrConfirmed { addr: &private },
        ));
        assert!(behaviour.pending_handlers.is_empty());
        let public = "/ip4/8.8.4.4/tcp/4001".parse().unwrap();
        // No preceding NewExternalAddrCandidate: Swarm can suppress that
        // notification for an address that has already been confirmed.
        behaviour.on_swarm_event(FromSwarm::ExternalAddrConfirmed(
            libp2p::swarm::behaviour::ExternalAddrConfirmed { addr: &public },
        ));
        assert!(behaviour.has_candidate);
        assert!(behaviour.deferred.is_empty());
        assert_eq!(behaviour.pending_handlers.len(), 1);
        assert_eq!(behaviour.attempts_by_peer[&peer], 1);
    }

    #[test]
    fn wan_only_dcutr_does_not_offer_private_routes() {
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3).with_lan_candidates(false);
        for addr in [
            "/ip4/192.168.137.59/udp/4001/quic-v1",
            "/ip4/10.0.2.15/udp/4001/quic-v1",
            "/ip4/172.17.0.1/udp/4001/quic-v1",
            "/ip4/100.64.0.1/udp/4001/quic-v1",
            "/ip6/fd00::1/udp/4001/quic-v1",
        ] {
            assert!(
                !behaviour.accept_candidate(&addr.parse().unwrap()),
                "{addr}"
            );
        }
        assert!(behaviour.accept_candidate(&"/ip4/8.8.8.8/udp/4001/quic-v1".parse().unwrap()));
    }

    #[test]
    fn dcutr_candidates_support_tcp_and_quic_without_protocol_suffixes() {
        let quic: Multiaddr = "/ip4/192.168.1.2/udp/4001/quic-v1".parse().unwrap();
        let tcp: Multiaddr = "/ip4/192.168.1.2/tcp/4001".parse().unwrap();
        let loopback: Multiaddr = "/ip4/127.0.0.1/udp/4001/quic-v1".parse().unwrap();

        assert!(is_dcutr_candidate(&quic));
        assert!(is_dcutr_candidate(&tcp));
        assert!(!is_dcutr_candidate(&loopback));
        for addr in [
            "/ip4/8.8.8.8/tcp/4001/ws",
            "/ip4/8.8.8.8/udp/4001/quic-v1/webtransport",
            "/ip4/8.8.8.8/tcp/0",
        ] {
            assert!(!is_dcutr_candidate(&addr.parse().unwrap()));
        }
    }

    #[test]
    fn fresh_nat_mappings_remain_eligible_after_initial_candidates_fill() {
        let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3)
            .with_lan_candidates(false);
        // Memory remains bounded by upstream's 20-entry LRU, rather than
        // rejecting every fresh mapping after the first eight observations.
        for port in 4001..4101 {
            for suffix in [format!("tcp/{port}"), format!("udp/{port}/quic-v1")] {
                let addr = format!("/ip4/8.8.8.8/{suffix}").parse().unwrap();
                assert!(behaviour.accept_candidate(&addr), "fresh mapping {addr}");
                behaviour.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
                    libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &addr },
                ));
            }
        }
        assert!(behaviour.accept_candidate(&"/ip4/9.9.9.9/tcp/4001".parse().unwrap()));
        assert!(!behaviour.accept_candidate(&"/ip4/192.168.1.2/tcp/4001".parse().unwrap()));
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
