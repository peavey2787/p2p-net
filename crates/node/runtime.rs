//! Long-running node runtime loop.
//!
//! Startup code constructs the swarm and initial discovery state, then hands
//! ownership to this module so `node::mod` stays focused on public startup.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::{IdentTopic, TopicHash};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::MissedTickBehavior;
use tokio::task::JoinHandle;

use crate::api::{AppMessage, NodeMetrics};
use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::DhtProviderState;
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::peer_cache::PeerCacheWriteBatch;
use crate::connectivity::relay::{RelayReservationPlan, RelayState};
use crate::connectivity::relay_discovery::RelaySelectionPlan;
use crate::connectivity::rendezvous::RendezvousState;
use crate::platform::NodeStorage;
use crate::protocol::pulse::HeartbeatReplayCache;
use crate::protocol::reputation::ReputationStore;
use crate::stack::{IdentifyAddressState, MeshBehaviour};

use super::commands::{self, NodeCommandContext};
use super::config::NodeConfig;
use super::dial::AutoDialStats;
use super::events::{self, ObservabilityBatch, SwarmEventContext};
use super::handle::NodeCommand;
use super::profile::ResolvedNodeConfig;
use super::public_ip;
use super::runtime_tasks::apply_public_ip_probe_result;
use super::snapshot::NodeSnapshot;

mod dht_schedule;
mod periodic;
mod state;

const OBSERVABILITY_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const PEER_CACHE_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
pub(crate) struct NodeRuntimeContext {
    pub(crate) cfg: NodeConfig,
    pub(crate) resolved_config: ResolvedNodeConfig,
    pub(crate) swarm: Swarm<MeshBehaviour>,
    pub(crate) local_peer: PeerId,
    pub(crate) heartbeat_topic: IdentTopic,
    pub(crate) snapshot: Arc<Mutex<NodeSnapshot>>,
    pub(crate) snapshot_revision: Arc<AtomicU64>,
    pub(crate) storage: Arc<dyn NodeStorage>,
    pub(crate) rendezvous_peers: Vec<Multiaddr>,
    pub(crate) relay_reservation_plan: RelayReservationPlan,
    pub(crate) relay_selection_plan: RelaySelectionPlan,
    pub(crate) rendezvous_state: RendezvousState,
    pub(crate) dht_state: DhtProviderState,
    pub(crate) peer_book: PeerBook,
    pub(crate) shutdown_rx: mpsc::Receiver<()>,
    pub(crate) command_rx: mpsc::Receiver<NodeCommand>,
    pub(crate) messages_tx: broadcast::Sender<AppMessage>,
}

pub(crate) fn spawn_node_runtime(ctx: NodeRuntimeContext) -> JoinHandle<()> {
    tokio::spawn(run_node_runtime(ctx))
}

async fn run_node_runtime(ctx: NodeRuntimeContext) {
    let NodeRuntimeContext {
        cfg,
        resolved_config,
        mut swarm,
        local_peer,
        heartbeat_topic,
        snapshot,
        snapshot_revision,
        storage,
        rendezvous_peers,
        relay_reservation_plan,
        relay_selection_plan,
        rendezvous_state,
        dht_state,
        peer_book,
        mut shutdown_rx,
        mut command_rx,
        messages_tx,
    } = ctx;

    let heartbeat_interval = Duration::from_secs(cfg.heartbeat_interval_secs.max(1));
    let mut ticker = tokio::time::interval(heartbeat_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut observability_ticker = tokio::time::interval(OBSERVABILITY_FLUSH_INTERVAL);
    observability_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut runtime_state = RuntimeState::new(
        &cfg,
        &resolved_config,
        relay_reservation_plan,
        relay_selection_plan,
        rendezvous_state,
        dht_state,
        peer_book,
    );
    let heartbeat_topic_hash = heartbeat_topic.hash().clone();
    let application_protocol_version = cfg
        .discovery
        .application_protocol_version(cfg.network_id)
        .expect("validated discovery namespace configuration");
    let application_namespaces = cfg
        .discovery
        .rendezvous_namespaces(cfg.network_id)
        .expect("validated discovery namespace configuration");
    let started_at = std::time::Instant::now();
    let enabled_listen_addresses = cfg
        .enabled_listen_addresses()
        .unwrap_or_default()
        .into_iter()
        .map(|addr| addr.to_string())
        .collect();
    let mut public_ip_probe = Box::pin(public_ip::probe_public_addresses(
        cfg.public_ip_probe.clone(),
        enabled_listen_addresses,
    ));
    let mut public_ip_probe_done = false;
    let mut dht_refresh_sleep = Box::pin(tokio::time::sleep_until(
        runtime_state.dht_refresh_schedule.next_due(),
    ));

    loop {
        tokio::select! {
            biased;
            _ = ticker.tick() => {
                periodic::tick_runtime(
                    &cfg,
                    &mut swarm,
                    local_peer,
                    &heartbeat_topic,
                    &snapshot,
                    &mut runtime_state,
                    started_at,
                ).await;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
            }
            _ = observability_ticker.tick() => {
                if periodic::flush_observability(
                    &cfg,
                    &snapshot,
                    storage.as_ref(),
                    &mut runtime_state,
                )
                .await
                {
                    snapshot_revision.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ = &mut dht_refresh_sleep => {
                periodic::refresh_dht(
                    &cfg,
                    &mut swarm,
                    &snapshot,
                    &mut runtime_state,
                    "scheduled",
                ).await;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
                dht_refresh_sleep
                    .as_mut()
                    .reset(runtime_state.dht_refresh_schedule.next_due());
            }
            public_ip_result = &mut public_ip_probe, if !public_ip_probe_done => {
                public_ip_probe_done = true;
                let refreshed_dht = !public_ip_result.external_addresses.is_empty();
                apply_public_ip_probe_result(
                    public_ip_result,
                    &cfg,
                    &mut swarm,
                    &snapshot,
                    &mut runtime_state.dht_state,
                    rendezvous_peers.len(),
                ).await;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
                if refreshed_dht {
                    runtime_state.dht_refresh_schedule.record_refresh();
                    dht_refresh_sleep
                        .as_mut()
                        .reset(runtime_state.dht_refresh_schedule.next_due());
                }
            }
            maybe_shutdown = shutdown_rx.recv() => {
                let _ = maybe_shutdown;
                break;
            }
            maybe_command = command_rx.recv() => {
                if let Some(command) = maybe_command {
                    commands::handle_node_command(
                        command,
                        NodeCommandContext {
                            swarm: &mut swarm,
                            local_peer,
                            network_id: cfg.network_id,
                            app_topic_hashes: &mut runtime_state.app_topic_hashes,
                            snapshot: &snapshot,
                            peer_book: &mut runtime_state.peer_book,
                            pending_connections: &mut runtime_state.pending_connections,
                            auto_dial_stats: &mut runtime_state.auto_dial_stats,
                            dcutr_policy: &cfg.dcutr,
                            metrics: &mut runtime_state.metrics,
                        },
                    )
                    .await;
                    snapshot_revision.fetch_add(1, Ordering::Relaxed);
                } else {
                    break;
                }
            }
            evt = swarm.select_next_some() => {
                let connectivity_recovered = matches!(
                    &evt,
                    SwarmEvent::ConnectionEstablished { num_established, .. }
                        if num_established.get() == 1
                            && swarm.connected_peers().take(2).count() == 1
                );
                let snapshot_update_deferred = events::snapshot_update_deferred(&evt);
                {
                    let mut event_ctx = SwarmEventContext {
                        snapshot: &snapshot,
                        rep: &mut runtime_state.rep,
                        relay_state: &mut runtime_state.relay_state,
                        rendezvous_state: &mut runtime_state.rendezvous_state,
                        dht_state: &mut runtime_state.dht_state,
                        peer_book: &mut runtime_state.peer_book,
                        pending_connections: &mut runtime_state.pending_connections,
                        auto_dial_stats: &mut runtime_state.auto_dial_stats,
                        connection_caps: &mut runtime_state.connection_caps,
                        relay_cfg: &cfg.relay,
                        dcutr_policy: &cfg.dcutr,
                        discovery_cfg: &cfg.discovery,
                        peer_cache_writes: &mut runtime_state.peer_cache_writes,
                        rendezvous_peers: &rendezvous_peers,
                        message_security: &cfg.message_security,
                        replay_cache: &mut runtime_state.replay_cache,
                        heartbeat_topic_hash: &heartbeat_topic_hash,
                        app_topic_hashes: &runtime_state.app_topic_hashes,
                        app_messages: &messages_tx,
                        metrics: &mut runtime_state.metrics,
                        identify_addresses: &mut runtime_state.identify_addresses,
                        observability: &mut runtime_state.observability,
                        local_peer,
                        network_id: cfg.network_id,
                        application_protocol_version: &application_protocol_version,
                        application_namespaces: &application_namespaces,
                    };
                    events::handle_swarm_event(evt, &mut swarm, &mut event_ctx).await;
                }
                if !snapshot_update_deferred {
                    snapshot_revision.fetch_add(1, Ordering::Relaxed);
                }
                if connectivity_recovered
                    && runtime_state
                        .dht_refresh_schedule
                        .request_connectivity_recovery_refresh()
                {
                    dht_refresh_sleep
                        .as_mut()
                        .reset(runtime_state.dht_refresh_schedule.next_due());
                }
            }
        }
    }
    runtime_state.flush_peer_cache(&cfg, storage.as_ref());
    periodic::flush_observability(&cfg, &snapshot, storage.as_ref(), &mut runtime_state).await;
}

struct RuntimeState {
    rep: ReputationStore,
    replay_cache: HeartbeatReplayCache,
    relay_state: RelayState,
    rendezvous_state: RendezvousState,
    dht_state: DhtProviderState,
    peer_book: PeerBook,
    pending_connections: PendingConnectionPlans,
    auto_dial_stats: AutoDialStats,
    connection_caps: ConnectionCapState,
    app_topic_hashes: Vec<TopicHash>,
    metrics: NodeMetrics,
    identify_addresses: IdentifyAddressState,
    observability: ObservabilityBatch,
    peer_cache_writes: PeerCacheWriteBatch,
    dht_refresh_schedule: dht_schedule::DhtRefreshSchedule,
    last_peer_cache_flush: std::time::Instant,
}
