use std::collections::VecDeque;
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
use crate::connectivity::peer_cache::PeerCacheWriteBatch;
use crate::connectivity::relay::{RelayServiceConfig, RelayState};
use crate::connectivity::rendezvous::RendezvousState;
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

#[derive(Debug, Default)]
pub(crate) struct ObservabilityBatch {
    app_messages_received: usize,
    app_messages_ignored: usize,
    app_messages_rejected: usize,
    gossip_messages_accepted: usize,
    gossip_messages_ignored: usize,
    gossip_messages_rejected: usize,
    peer_connectivity_dirty: bool,
    dht_snapshot_dirty: bool,
    pulses: VecDeque<String>,
}

impl ObservabilityBatch {
    const MAX_PENDING_PULSES: usize = 64;

    pub(crate) fn app_received(&mut self) {
        self.app_messages_received = self.app_messages_received.saturating_add(1);
    }

    pub(crate) fn app_ignored(&mut self) {
        self.app_messages_ignored = self.app_messages_ignored.saturating_add(1);
    }

    pub(crate) fn app_rejected(&mut self) {
        self.app_messages_rejected = self.app_messages_rejected.saturating_add(1);
    }

    pub(crate) fn gossip_accepted(&mut self, peer_connectivity_dirty: bool) {
        self.gossip_messages_accepted = self.gossip_messages_accepted.saturating_add(1);
        self.peer_connectivity_dirty |= peer_connectivity_dirty;
    }

    pub(crate) fn gossip_ignored(&mut self) {
        self.gossip_messages_ignored = self.gossip_messages_ignored.saturating_add(1);
    }

    pub(crate) fn gossip_rejected(&mut self) {
        self.gossip_messages_rejected = self.gossip_messages_rejected.saturating_add(1);
    }

    pub(crate) fn dht_dirty(&mut self) {
        self.dht_snapshot_dirty = true;
    }

    pub(crate) fn peer_connectivity_dirty(&mut self) {
        self.peer_connectivity_dirty = true;
    }

    pub(crate) fn pulse(&mut self, line: String) {
        if self.pulses.len() >= Self::MAX_PENDING_PULSES {
            let _ = self.pulses.pop_front();
        }
        self.pulses.push_back(line);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.app_messages_received == 0
            && self.app_messages_ignored == 0
            && self.app_messages_rejected == 0
            && self.gossip_messages_accepted == 0
            && self.gossip_messages_ignored == 0
            && self.gossip_messages_rejected == 0
            && !self.peer_connectivity_dirty
            && !self.dht_snapshot_dirty
            && self.pulses.is_empty()
    }
}

pub(crate) fn flush_observability_snapshot(
    snapshot: &mut NodeSnapshot,
    batch: &mut ObservabilityBatch,
    dht_state: &DhtProviderState,
    peer_book: &PeerBook,
    auto_dial_stats: &AutoDialStats,
    pending_connections: &PendingConnectionPlans,
    auto_connect_enabled: bool,
) {
    snapshot.app_messages_received = snapshot
        .app_messages_received
        .saturating_add(batch.app_messages_received);
    snapshot.app_messages_ignored = snapshot
        .app_messages_ignored
        .saturating_add(batch.app_messages_ignored);
    snapshot.app_messages_rejected = snapshot
        .app_messages_rejected
        .saturating_add(batch.app_messages_rejected);
    snapshot.gossip_messages_accepted = snapshot
        .gossip_messages_accepted
        .saturating_add(batch.gossip_messages_accepted);
    snapshot.gossip_messages_ignored = snapshot
        .gossip_messages_ignored
        .saturating_add(batch.gossip_messages_ignored);
    snapshot.gossip_messages_rejected = snapshot
        .gossip_messages_rejected
        .saturating_add(batch.gossip_messages_rejected);
    if batch.dht_snapshot_dirty {
        snapshot.dht_provider_announce_attempts = dht_state.announce_attempts;
        snapshot.dht_provider_announce_failures = dht_state.announce_failures;
        snapshot.dht_provider_namespaces_announced = dht_state.namespaces_announced.len();
        snapshot.dht_provider_queries = dht_state.provider_queries;
        snapshot.dht_provider_query_failures = dht_state.provider_query_failures;
        snapshot.dht_provider_records_found = dht_state.provider_records_found;
        snapshot.dht_provider_queries_finished = dht_state.provider_queries_finished;
        snapshot.dht_provider_peers_discovered = dht_state.provider_peer_count();
    }
    if batch.peer_connectivity_dirty {
        sync_peer_connectivity_fields(
            snapshot,
            peer_book,
            auto_dial_stats,
            pending_connections,
            auto_connect_enabled,
        );
    }
    for line in batch.pulses.drain(..) {
        super::push_pulse(&mut snapshot.pulses, line);
    }
    *batch = ObservabilityBatch::default();
}

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
    pub(crate) peer_cache_writes: &'a mut PeerCacheWriteBatch,
    pub(crate) rendezvous_peers: &'a [Multiaddr],
    pub(crate) message_security: &'a MessageSecurityConfig,
    pub(crate) replay_cache: &'a mut HeartbeatReplayCache,
    pub(crate) heartbeat_topic_hash: &'a TopicHash,
    pub(crate) app_topic_hashes: &'a [TopicHash],
    pub(crate) app_messages: &'a broadcast::Sender<AppMessage>,
    pub(crate) metrics: &'a mut NodeMetrics,
    pub(crate) identify_addresses: &'a mut IdentifyAddressState,
    pub(crate) observability: &'a mut ObservabilityBatch,
    pub(crate) local_peer: PeerId,
    pub(crate) network_id: u32,
    pub(crate) application_protocol_version: &'a str,
    pub(crate) application_namespaces: &'a [String],
}

pub(crate) fn sync_peer_connectivity_snapshot(
    snapshot: &mut NodeSnapshot,
    ctx: &SwarmEventContext<'_>,
) {
    sync_peer_connectivity_fields(
        snapshot,
        ctx.peer_book,
        ctx.auto_dial_stats,
        ctx.pending_connections,
        ctx.discovery_cfg.public_bootstrap.auto_connect_discovered_peers,
    );
}

pub(crate) fn sync_peer_connectivity_fields(
    snapshot: &mut NodeSnapshot,
    peer_book: &PeerBook,
    auto_dial_stats: &AutoDialStats,
    pending_connections: &PendingConnectionPlans,
    auto_connect_enabled: bool,
) {
    snapshot.peer_book_known_peers = peer_book.len();
    snapshot.peer_book_discovered_peers = peer_book.discovered_count();
    snapshot.auto_connect_enabled = auto_connect_enabled;
    snapshot.auto_connect_dial_attempts = auto_dial_stats.dial_attempts;
    snapshot.auto_connect_dial_failures = auto_dial_stats.dial_failures;
    snapshot.auto_connect_awaiting_address_peers = auto_dial_stats.awaiting_address_count();
    snapshot.connection_plan_pending_peers = pending_connections.pending_count();
}

pub(crate) fn sync_swarm_connection_snapshot(
    snapshot: &mut NodeSnapshot,
    swarm: &Swarm<MeshBehaviour>,
    ctx: &SwarmEventContext<'_>,
) {
    sync_peer_connectivity_snapshot(snapshot, ctx);

    let swarm_peers = swarm.connected_peers().copied().collect::<Vec<_>>();
    let application_swarm_peers = swarm_peers
        .iter()
        .copied()
        .filter(|peer| {
            ctx.peer_book
                .has_application_namespace(peer, ctx.application_namespaces)
        })
        .collect::<std::collections::HashSet<_>>();
    let relay_swarm_peers = swarm_peers
        .iter()
        .copied()
        .filter(|peer| !application_swarm_peers.contains(peer))
        .filter(|peer| is_relay_infrastructure_peer(*peer, ctx))
        .collect::<std::collections::HashSet<_>>();

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
pub(crate) fn snapshot_update_deferred(evt: &SwarmEvent<MeshEvent>) -> bool {
    matches!(
        evt,
        SwarmEvent::Behaviour(MeshEvent::Kademlia(_))
            | SwarmEvent::Behaviour(MeshEvent::Gossipsub(_))
    )
}

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
                connection::EstablishedConnection {
                    peer_id,
                    connection_id,
                    remote_addr,
                    relayed_endpoint,
                    outgoing,
                    endpoint_debug,
                },
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
            kademlia::handle_event(swarm, &ev, ctx);
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
            );
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
            app::handle_app_message(propagation_source, message.data, ctx);
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
                ctx,
            );
        }
        SwarmEvent::Behaviour(MeshEvent::Identify(ev)) => {
            connection::handle_identify_observed_addr(swarm, &ev, ctx).await;
            let event = MeshEvent::Identify(ev);
            on_mesh_event(
                swarm,
                &event,
                ctx.discovery_cfg,
                ctx.peer_cache_writes,
                ctx.peer_book,
                ctx.identify_addresses,
            );
        }
        SwarmEvent::Behaviour(ev) => {
            on_mesh_event(
                swarm,
                &ev,
                ctx.discovery_cfg,
                ctx.peer_cache_writes,
                ctx.peer_book,
                ctx.identify_addresses,
            );
        }
        _ => {}
    }
}
