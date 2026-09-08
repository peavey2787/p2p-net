//! Reserve admission headroom for application peers without raising hard caps.

use std::collections::{HashSet, VecDeque};
use std::convert::Infallible;
use std::task::{Context, Poll};

use libp2p::core::{transport::PortUse, Endpoint};
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{connection_limits, Multiaddr, PeerId};

use crate::connectivity::limits::ConnectionLimitsConfig;

const MAX_PRIORITY_PEERS: usize = 1024;

/// One limiter tracks every connection. A lower infrastructure admission ceiling
/// keeps unrelated infrastructure from consuming all outbound slots while an
/// application dial or DCUtR handshake is in flight. Public relays remain usable
/// as infrastructure; they are not required to match an application namespace.
pub struct PrioritizedConnectionLimits {
    hard: connection_limits::Behaviour,
    hard_limits: connection_limits::ConnectionLimits,
    infrastructure_limits: connection_limits::ConnectionLimits,
    priority_peers: HashSet<PeerId>,
    priority_order: VecDeque<PeerId>,
}

fn infrastructure_ceiling(limit: Option<u32>) -> Option<u32> {
    limit.map(|limit| limit.saturating_sub((limit / 4).clamp(1, 8)).max(1))
}

impl PrioritizedConnectionLimits {
    pub fn new(config: &ConnectionLimitsConfig) -> Self {
        let mut infrastructure = config.clone();
        infrastructure.max_pending_outgoing = infrastructure_ceiling(config.max_pending_outgoing);
        infrastructure.max_established_outgoing =
            infrastructure_ceiling(config.max_established_outgoing);
        infrastructure.max_established = infrastructure_ceiling(config.max_established);
        Self {
            hard: connection_limits::Behaviour::new(config.to_libp2p_limits()),
            hard_limits: config.to_libp2p_limits(),
            infrastructure_limits: infrastructure.to_libp2p_limits(),
            priority_peers: HashSet::new(),
            priority_order: VecDeque::new(),
        }
    }

    pub fn allow_application_peer(&mut self, peer: PeerId) {
        if !self.priority_peers.insert(peer) {
            return;
        }
        self.priority_order.push_back(peer);
        while self.priority_order.len() > MAX_PRIORITY_PEERS {
            if let Some(old) = self.priority_order.pop_front() {
                self.priority_peers.remove(&old);
            }
        }
    }

    pub fn release_application_peer(&mut self, peer: &PeerId) {
        self.priority_peers.remove(peer);
        self.priority_order.retain(|known| known != peer);
    }

    fn select_admission_limits(&mut self, peer: Option<PeerId>) {
        // Swarm serializes these hooks through &mut self. Selecting the ceiling
        // for each admission reuses a single set of connection counters instead
        // of keeping a duplicate peer-accounting system or bypassing hard caps.
        *self.hard.limits_mut() = if peer.is_some_and(|peer| self.priority_peers.contains(&peer)) {
            self.hard_limits.clone()
        } else {
            self.infrastructure_limits.clone()
        };
    }
}

impl NetworkBehaviour for PrioritizedConnectionLimits {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = Infallible;

    fn handle_pending_inbound_connection(
        &mut self,
        id: ConnectionId,
        local: &Multiaddr,
        remote: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        self.select_admission_limits(None);
        self.hard
            .handle_pending_inbound_connection(id, local, remote)
    }

    fn handle_pending_outbound_connection(
        &mut self,
        id: ConnectionId,
        peer: Option<PeerId>,
        addresses: &[Multiaddr],
        role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        self.select_admission_limits(peer);
        self.hard
            .handle_pending_outbound_connection(id, peer, addresses, role)
    }

    fn handle_established_inbound_connection(
        &mut self,
        id: ConnectionId,
        peer: PeerId,
        local: &Multiaddr,
        remote: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.select_admission_limits(Some(peer));
        self.hard
            .handle_established_inbound_connection(id, peer, local, remote)
    }

    fn handle_established_outbound_connection(
        &mut self,
        id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role: Endpoint,
        port: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.select_admission_limits(Some(peer));
        self.hard
            .handle_established_outbound_connection(id, peer, addr, role, port)
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        self.hard.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        libp2p::core::util::unreachable(event)
    }

    fn poll(&mut self, _: &mut Context<'_>) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::core::ConnectedPoint;
    use libp2p::swarm::behaviour::ConnectionEstablished;

    #[test]
    fn application_headroom_preserves_the_hard_cap() {
        let config = ConnectionLimitsConfig {
            max_established_outgoing: Some(4),
            ..Default::default()
        };
        let mut limits = PrioritizedConnectionLimits::new(&config);
        let addr: Multiaddr = "/ip4/8.8.8.8/udp/4001/quic-v1".parse().unwrap();
        let endpoint = ConnectedPoint::Dialer {
            address: addr.clone(),
            role_override: Endpoint::Dialer,
            port_use: PortUse::Reuse,
        };
        for n in 0..3 {
            let id = ConnectionId::new_unchecked(n);
            let peer = PeerId::random();
            limits
                .handle_established_outbound_connection(
                    id,
                    peer,
                    &addr,
                    Endpoint::Dialer,
                    PortUse::Reuse,
                )
                .unwrap();
            limits.on_swarm_event(FromSwarm::ConnectionEstablished(ConnectionEstablished {
                peer_id: peer,
                connection_id: id,
                endpoint: &endpoint,
                failed_addresses: &[],
                other_established: 0,
            }));
        }
        let id = ConnectionId::new_unchecked(3);
        let app = PeerId::random();
        assert!(limits
            .handle_established_outbound_connection(
                id,
                app,
                &addr,
                Endpoint::Dialer,
                PortUse::Reuse
            )
            .is_err());
        limits.allow_application_peer(app);
        limits
            .handle_established_outbound_connection(
                id,
                app,
                &addr,
                Endpoint::Dialer,
                PortUse::Reuse,
            )
            .unwrap();
        limits.on_swarm_event(FromSwarm::ConnectionEstablished(ConnectionEstablished {
            peer_id: app,
            connection_id: id,
            endpoint: &endpoint,
            failed_addresses: &[],
            other_established: 0,
        }));
        let other_app = PeerId::random();
        limits.allow_application_peer(other_app);
        assert!(limits
            .handle_established_outbound_connection(
                ConnectionId::new_unchecked(4),
                other_app,
                &addr,
                Endpoint::Dialer,
                PortUse::Reuse
            )
            .is_err());
    }
}
