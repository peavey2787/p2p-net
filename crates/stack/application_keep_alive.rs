//! Per-application-peer connection retention without retaining public infrastructure.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use libp2p::core::{transport::PortUse, upgrade::DeniedUpgrade, Endpoint};
use libp2p::swarm::handler::{
    ConnectionEvent, DialUpgradeError, FullyNegotiatedInbound, FullyNegotiatedOutbound,
};
use libp2p::swarm::{
    ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent, ConnectionId, FromSwarm,
    NetworkBehaviour, StreamUpgradeError, SubstreamProtocol, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{Multiaddr, PeerId};

/// Keeps only peers that passed application compatibility verification alive.
///
/// Every connection initially receives a disabled flag. Identify, namespace
/// discovery, or an authenticated application heartbeat enables the flag for
/// every connection to that peer. Public DHT/bootstrap/relay infrastructure
/// therefore remains governed by the normal swarm idle timeout.
#[derive(Default)]
pub struct ApplicationKeepAlive {
    peers: HashMap<PeerId, PeerRetention>,
}

#[derive(Default)]
struct PeerRetention {
    verified: bool,
    active_direct: Option<ConnectionId>,
    active_relay: Option<ConnectionId>,
    connections: HashMap<ConnectionId, RetainedConnection>,
}

struct RetainedConnection {
    keep_alive: Arc<AtomicBool>,
    relayed: bool,
}

impl ApplicationKeepAlive {
    pub fn allow_peer(&mut self, peer: PeerId) {
        let retention = self.peers.entry(peer).or_default();
        retention.verified = true;
        select_retained_connection(retention);
    }

    pub fn release_peer(&mut self, peer: &PeerId) {
        if let Some(retention) = self.peers.get_mut(peer) {
            retention.verified = false;
            retention.active_direct = None;
            retention.active_relay = None;
            for connection in retention.connections.values() {
                connection.keep_alive.store(false, Ordering::Release);
            }
            if retention.connections.is_empty() {
                self.peers.remove(peer);
            }
        }
    }

    fn handler_for(
        &mut self,
        peer: PeerId,
        connection_id: ConnectionId,
        relayed: bool,
    ) -> ApplicationKeepAliveHandler {
        let keep_alive = Arc::new(AtomicBool::new(false));
        let retention = self.peers.entry(peer).or_default();
        retention.connections.insert(
            connection_id,
            RetainedConnection {
                keep_alive: Arc::clone(&keep_alive),
                relayed,
            },
        );
        select_retained_connection(retention);
        ApplicationKeepAliveHandler { keep_alive }
    }
}

fn select_retained_connection(retention: &mut PeerRetention) {
    if !retention.verified {
        retention.active_direct = None;
        retention.active_relay = None;
    } else {
        let active_direct_is_valid = retention.active_direct.is_some_and(|connection_id| {
            retention
                .connections
                .get(&connection_id)
                .is_some_and(|connection| !connection.relayed)
        });
        if !active_direct_is_valid {
            retention.active_direct = retention
                .connections
                .iter()
                .find_map(|(id, connection)| (!connection.relayed).then_some(*id));
        }
        let active_relay_is_valid = retention.active_relay.is_some_and(|connection_id| {
            retention
                .connections
                .get(&connection_id)
                .is_some_and(|connection| connection.relayed)
        });
        if !active_relay_is_valid {
            retention.active_relay = retention
                .connections
                .iter()
                .find_map(|(id, connection)| connection.relayed.then_some(*id));
        }
    }

    for (connection_id, connection) in &retention.connections {
        let retained = retention.active_direct == Some(*connection_id)
            || retention.active_relay == Some(*connection_id);
        connection.keep_alive.store(retained, Ordering::Release);
    }
}

fn is_relayed(addrs: &[&Multiaddr]) -> bool {
    addrs.iter().any(|addr| {
        addr.iter()
            .any(|protocol| matches!(protocol, libp2p::multiaddr::Protocol::P2pCircuit))
    })
}

impl NetworkBehaviour for ApplicationKeepAlive {
    type ConnectionHandler = ApplicationKeepAliveHandler;
    type ToSwarm = Infallible;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(self.handler_for(peer, connection_id, is_relayed(&[local_addr, remote_addr])))
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        remote_addr: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(self.handler_for(peer, connection_id, is_relayed(&[remote_addr])))
    }

    fn on_swarm_event(&mut self, event: FromSwarm<'_>) {
        if let FromSwarm::ConnectionClosed(event) = event {
            let remove_peer = if let Some(retention) = self.peers.get_mut(&event.peer_id) {
                retention.connections.remove(&event.connection_id);
                if event.remaining_established == 0 || retention.connections.is_empty() {
                    true
                } else {
                    if retention.active_direct == Some(event.connection_id) {
                        retention.active_direct = None;
                    }
                    if retention.active_relay == Some(event.connection_id) {
                        retention.active_relay = None;
                    }
                    select_retained_connection(retention);
                    false
                }
            } else {
                false
            };
            if remove_peer {
                self.peers.remove(&event.peer_id);
            }
        }
    }

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        libp2p::core::util::unreachable(event);
    }

    fn poll(&mut self, _: &mut Context<'_>) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Pending
    }
}

/// Protocol-free handler whose only responsibility is the connection lifetime.
pub struct ApplicationKeepAliveHandler {
    keep_alive: Arc<AtomicBool>,
}

impl ConnectionHandler for ApplicationKeepAliveHandler {
    type FromBehaviour = Infallible;
    type ToBehaviour = Infallible;
    type InboundProtocol = DeniedUpgrade;
    type OutboundProtocol = DeniedUpgrade;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol> {
        SubstreamProtocol::new(DeniedUpgrade, ())
    }

    fn connection_keep_alive(&self) -> bool {
        self.keep_alive.load(Ordering::Acquire)
    }

    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        libp2p::core::util::unreachable(event);
    }

    fn poll(
        &mut self,
        _: &mut Context<'_>,
    ) -> Poll<ConnectionHandlerEvent<Self::OutboundProtocol, (), Self::ToBehaviour>> {
        Poll::Pending
    }

    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<Self::InboundProtocol, Self::OutboundProtocol>,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol, ..
            }) => libp2p::core::util::unreachable(protocol),
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol, ..
            }) => libp2p::core::util::unreachable(protocol),
            ConnectionEvent::DialUpgradeError(DialUpgradeError { error, .. }) => match error {
                StreamUpgradeError::Timeout => unreachable!(),
                StreamUpgradeError::Apply(error) => libp2p::core::util::unreachable(error),
                StreamUpgradeError::NegotiationFailed | StreamUpgradeError::Io(_) => {
                    unreachable!("DeniedUpgrade does not support protocols")
                }
            },
            ConnectionEvent::AddressChange(_)
            | ConnectionEvent::ListenUpgradeError(_)
            | ConnectionEvent::LocalProtocolsChange(_)
            | ConnectionEvent::RemoteProtocolsChange(_) => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_updates_existing_connection_handlers() {
        let peer = PeerId::random();
        let mut behaviour = ApplicationKeepAlive::default();
        let handler = behaviour.handler_for(peer, ConnectionId::new_unchecked(1), false);

        assert!(!handler.connection_keep_alive());
        behaviour.allow_peer(peer);
        assert!(handler.connection_keep_alive());
        behaviour.release_peer(&peer);
        assert!(!handler.connection_keep_alive());
        assert_eq!(behaviour.peers.len(), 1);
    }

    #[test]
    fn only_one_direct_connection_to_a_verified_peer_is_retained() {
        let peer = PeerId::random();
        let mut behaviour = ApplicationKeepAlive::default();
        let first = behaviour.handler_for(peer, ConnectionId::new_unchecked(1), false);
        let second = behaviour.handler_for(peer, ConnectionId::new_unchecked(2), false);

        behaviour.allow_peer(peer);

        assert_ne!(
            first.connection_keep_alive(),
            second.connection_keep_alive()
        );
        assert_eq!(behaviour.peers.len(), 1);
    }

    #[test]
    fn direct_connection_keeps_relay_fallback_retained() {
        let peer = PeerId::random();
        let mut behaviour = ApplicationKeepAlive::default();
        let relay = behaviour.handler_for(peer, ConnectionId::new_unchecked(1), true);
        behaviour.allow_peer(peer);
        assert!(relay.connection_keep_alive());

        let direct = behaviour.handler_for(peer, ConnectionId::new_unchecked(2), false);

        assert!(relay.connection_keep_alive());
        assert!(direct.connection_keep_alive());
    }
}
