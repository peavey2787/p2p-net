use std::collections::BTreeSet;
use std::sync::Arc;

use libp2p::gossipsub::TopicHash;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use tokio::sync::{broadcast, Mutex};

use crate::api::{AppMessage, NodeMetrics, PeerSource};
use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dcutr::DcutrPolicy;
use crate::connectivity::dht::DhtProviderState;
use crate::connectivity::discovery::DiscoveryConfig;
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::relay::{RelayServiceConfig, RelayState};
use crate::connectivity::rendezvous::RendezvousState;
use crate::platform::NodeStorage;
use crate::protocol::pulse::{HeartbeatReplayCache, MessageSecurityConfig};
use crate::protocol::reputation::ReputationStore;
use crate::stack::{on_mesh_event, IdentifyAddressState, MeshBehaviour, MeshEvent};

use super::dial::AutoDialStats;
use super::snapshot::NodeSnapshot;

mod app;
mod connection;
mod dcutr;
mod gossip;
mod kademlia;
mod relay_client;
mod relay_server;
mod rendezvous;

pub(crate) use relay_server::enforce_relay_schedule;

pub(crate) struct SwarmEventContext<'a> {
    pub(crate) snapshot: &'a Arc<Mutex<NodeSnapshot>>,
    pub(crate) rep: &'a mut ReputationStore,
    pub(crate) relay_state: &'a mut RelayState,
    pub(crate) rendezvous_state: &'a mut RendezvousState,
    pub(crate) dht_state: &'a mut DhtProviderState,
    pub(crate) peer_book: &'a mut PeerBook,
    pub(crate) pending_connections: &'a mut PendingConnectionPlans,
    pub(crate) auto_dial_stats: &'a mut AutoDialStats,
    pub(crate) connection_caps: &'a mut ConnectionCapState,
    pub(crate) relay_cfg: &'a RelayServiceConfig,
    pub(crate) dcutr_policy: &'a DcutrPolicy,
    pub(crate) discovery_cfg: &'a DiscoveryConfig,
    pub(crate) storage: &'a dyn NodeStorage,
    pub(crate) rendezvous_peers: &'a [Multiaddr],
    pub(crate) message_security: &'a MessageSecurityConfig,
    pub(crate) replay_cache: &'a mut HeartbeatReplayCache,
    pub(crate) heartbeat_topic_hash: &'a TopicHash,
    pub(crate) app_topic_hashes: &'a [TopicHash],
    pub(crate) app_messages: &'a broadcast::Sender<AppMessage>,
    pub(crate) metrics: &'a mut NodeMetrics,
    pub(crate) identify_addresses: &'a mut IdentifyAddressState,
    pub(crate) local_peer: PeerId,
    pub(crate) network_id: u32,
}

pub(crate) fn sync_peer_connectivity_snapshot(
    snapshot: &mut NodeSnapshot,
    ctx: &SwarmEventContext<'_>,
) {
    snapshot.peer_book_known_peers = ctx.peer_book.len();
    snapshot.peer_book_discovered_peers = ctx.peer_book.discovered_count();
    snapshot.auto_connect_enabled = ctx
        .discovery_cfg
        .public_bootstrap
        .auto_connect_discovered_peers;
    snapshot.auto_connect_dial_attempts = ctx.auto_dial_stats.dial_attempts;
    snapshot.auto_connect_dial_failures = ctx.auto_dial_stats.dial_failures;
    snapshot.auto_connect_awaiting_address_peers = ctx.auto_dial_stats.awaiting_address_count();
    snapshot.connection_plan_pending_peers = ctx.pending_connections.pending_count();
}

pub(crate) fn sync_swarm_connection_snapshot(
    snapshot: &mut NodeSnapshot,
    swarm: &Swarm<MeshBehaviour>,
    ctx: &SwarmEventContext<'_>,
) {
    sync_peer_connectivity_snapshot(snapshot, ctx);

    let application_namespaces = snapshot
        .discovery_namespaces
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let swarm_peers = swarm.connected_peers().copied().collect::<Vec<_>>();
    let application_swarm_peers = swarm_peers
        .iter()
        .copied()
        .filter(|peer| {
            ctx.peer_book
                .has_application_namespace(peer, &application_namespaces)
        })
        .collect::<BTreeSet<_>>();
    let relay_swarm_peers = swarm_peers
        .iter()
        .copied()
        .filter(|peer| !application_swarm_peers.contains(peer))
        .filter(|peer| is_relay_infrastructure_peer(*peer, ctx))
        .collect::<BTreeSet<_>>();

    let all_swarm = swarm_peers.len();
    let infrastructure = all_swarm.saturating_sub(application_swarm_peers.len());
    let relay = relay_swarm_peers.len();

    snapshot.connected_peers = all_swarm;
    snapshot.all_swarm_connections = all_swarm;
    snapshot.application_peer_connections = application_swarm_peers.len();
    snapshot.infrastructure_peer_connections = infrastructure;
    snapshot.relay_peer_connections = relay;
    snapshot.dht_routing_peer_connections = infrastructure.saturating_sub(relay);
}

fn is_relay_infrastructure_peer(peer: PeerId, ctx: &SwarmEventContext<'_>) -> bool {
    if ctx.relay_state.relay_client_reservations.contains(&peer)
        || ctx.relay_state.relay_client_attempted_peers.contains(&peer)
    {
        return true;
    }
    ctx.peer_book.record(&peer).is_some_and(|record| {
        record.relay_preferred
            || record.supports_relay == Some(true)
            || record.sources.contains(&PeerSource::RelayDiscovery)
            || record.sources.contains(&PeerSource::PublicRelayDiscovery)
    })
}

/// Top-level swarm dispatch only. Responsibility-specific event handling lives in
/// the child modules under `node/events/` so relay, DCUtR, rendezvous, gossip,
/// and connection policy can evolve without turning this dispatcher into a god file.
pub(crate) async fn handle_swarm_event(
    evt: SwarmEvent<MeshEvent>,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    match evt {
        SwarmEvent::ConnectionEstablished {
            peer_id,
            connection_id,
            endpoint,
            ..
        } => {
            let remote_addr = endpoint.get_remote_address().clone();
            let relayed_endpoint = endpoint.is_relayed();
            let outgoing = endpoint.is_dialer();
            let endpoint_debug = format!("{endpoint:?}");
            connection::handle_connection_established(
                peer_id,
                connection_id,
                remote_addr,
                relayed_endpoint,
                outgoing,
                endpoint_debug,
                swarm,
                ctx,
            )
            .await;
        }
        SwarmEvent::ConnectionClosed {
            peer_id,
            connection_id,
            ..
        } => {
            connection::handle_connection_closed(peer_id, connection_id, swarm, ctx).await;
        }
        SwarmEvent::IncomingConnectionError { error, peer_id, .. } => {
            connection::handle_incoming_connection_error(
                format!("{peer_id:?}"),
                format!("{error:?}"),
                ctx,
            )
            .await;
        }
        SwarmEvent::OutgoingConnectionError { error, peer_id, .. } => {
            connection::handle_outgoing_connection_error(peer_id, format!("{error:?}"), swarm, ctx)
                .await;
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            connection::handle_new_listen_addr(address, swarm, ctx).await;
        }
        SwarmEvent::ExpiredListenAddr { address, .. } => {
            connection::handle_expired_listen_addr(address, swarm, ctx).await;
        }
        SwarmEvent::ListenerError { error, .. } => {
            connection::handle_listener_error(format!("{error:?}"), ctx).await;
        }
        SwarmEvent::Behaviour(MeshEvent::AutoNat(ev)) => {
            connection::handle_autonat_event(ev, ctx).await;
        }
        SwarmEvent::Behaviour(MeshEvent::RelayClient(ev)) => {
            relay_client::handle_event(ev, swarm, ctx.snapshot, ctx.relay_state).await;
        }
        SwarmEvent::Behaviour(MeshEvent::RelayServer(ev)) => {
            relay_server::handle_event(ev, ctx.snapshot, ctx.relay_state).await;
        }
        SwarmEvent::Behaviour(MeshEvent::Dcutr(ev)) => {
            dcutr::handle_event(ev, ctx.snapshot, ctx.relay_state, ctx.dcutr_policy).await;
        }
        SwarmEvent::Behaviour(MeshEvent::RendezvousClient(ev)) => {
            rendezvous::handle_client_event(swarm, &ev, ctx).await;
        }
        SwarmEvent::Behaviour(MeshEvent::RendezvousServer(ev)) => {
            rendezvous::handle_server_event(&ev, ctx).await;
        }
        SwarmEvent::Behaviour(MeshEvent::Kademlia(ev)) => {
            kademlia::handle_event(swarm, &ev, ctx).await;
        }
        SwarmEvent::Behaviour(MeshEvent::Gossipsub(libp2p::gossipsub::Event::Message {
            propagation_source,
            message,
            message_id,
        })) if message.topic == *ctx.heartbeat_topic_hash => {
            gossip::handle_heartbeat_message(
                swarm,
                propagation_source,
                message_id,
                message.data,
                ctx,
            )
            .await;
        }
        SwarmEvent::Behaviour(MeshEvent::Gossipsub(libp2p::gossipsub::Event::Message {
            propagation_source,
            message,
            ..
        })) if ctx
            .app_topic_hashes
            .iter()
            .any(|topic| topic == &message.topic) =>
        {
            app::handle_app_message(propagation_source, message.data, ctx).await;
        }
        SwarmEvent::Behaviour(MeshEvent::Gossipsub(libp2p::gossipsub::Event::Message {
            propagation_source,
            message_id,
            ..
        })) => {
            gossip::handle_unexpected_topic_message(
                swarm,
                propagation_source,
                message_id,
                ctx.snapshot,
            )
            .await;
        }
        SwarmEvent::Behaviour(MeshEvent::Identify(ev)) => {
            connection::handle_identify_observed_addr(swarm, &ev, ctx).await;
            let event = MeshEvent::Identify(ev);
            on_mesh_event(
                swarm,
                &event,
                ctx.discovery_cfg,
                ctx.storage,
                ctx.peer_book,
                ctx.identify_addresses,
            );
        }
        SwarmEvent::Behaviour(ev) => {
            on_mesh_event(
                swarm,
                &ev,
                ctx.discovery_cfg,
                ctx.storage,
                ctx.peer_book,
                ctx.identify_addresses,
            );
        }
        _ => {}
    }
}
