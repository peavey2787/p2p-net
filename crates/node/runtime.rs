//! Long-running node runtime loop.
//!
//! Startup code constructs the swarm and initial discovery state, then hands
//! ownership to this module so `node::mod` stays focused on public startup.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use libp2p::gossipsub::{IdentTopic, TopicHash};
use libp2p::{Multiaddr, PeerId, Swarm};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::api::{AppMessage, NodeMetrics};
use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::{start_dht_namespace_discovery_with_interval, DhtProviderState};
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::peer_book::PeerBook;
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
use super::events::{self, SwarmEventContext};
use super::handle::NodeCommand;
use super::profile::ResolvedNodeConfig;
use super::public_ip;
use super::runtime_maintenance;
use super::runtime_tasks::{
    apply_dht_refresh_snapshot, apply_public_ip_probe_result, publish_heartbeat,
};
use super::snapshot::NodeSnapshot;

const DHT_STARTUP_REFRESH_INTERVAL_SECS: u64 = 5;
const DHT_STARTUP_REFRESHES: usize = 12;
pub(crate) struct NodeRuntimeContext {
    pub(crate) cfg: NodeConfig,
    pub(crate) resolved_config: ResolvedNodeConfig,
    pub(crate) swarm: Swarm<MeshBehaviour>,
    pub(crate) local_peer: PeerId,
    pub(crate) heartbeat_topic: IdentTopic,
    pub(crate) snapshot: Arc<Mutex<NodeSnapshot>>,
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
    let started_at = std::time::Instant::now();
    let mut public_ip_probe = Box::pin(public_ip::probe_public_addresses(
        cfg.public_ip_probe.clone(),
        cfg.listen_addresses.clone(),
    ));
    let mut public_ip_probe_done = false;

    loop {
        tokio::select! {
            biased;
            _ = ticker.tick() => {
                tick_runtime(
                    &cfg,
                    &mut swarm,
                    local_peer,
                    &heartbeat_topic,
                    &snapshot,
                    &mut runtime_state,
                    started_at,
                ).await;
            }
            public_ip_result = &mut public_ip_probe, if !public_ip_probe_done => {
                public_ip_probe_done = true;
                apply_public_ip_probe_result(
                    public_ip_result,
                    &cfg,
                    &mut swarm,
                    &snapshot,
                    &mut runtime_state.dht_state,
                    rendezvous_peers.len(),
                ).await;
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
                } else {
                    break;
                }
            }
            evt = swarm.select_next_some() => {
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
                    storage: storage.as_ref(),
                    rendezvous_peers: &rendezvous_peers,
                    message_security: &cfg.message_security,
                    replay_cache: &mut runtime_state.replay_cache,
                    heartbeat_topic_hash: &heartbeat_topic_hash,
                    app_topic_hashes: &runtime_state.app_topic_hashes,
                    app_messages: &messages_tx,
                    metrics: &mut runtime_state.metrics,
                    identify_addresses: &mut runtime_state.identify_addresses,
                    local_peer,
                    network_id: cfg.network_id,
                };
                events::handle_swarm_event(evt, &mut swarm, &mut event_ctx).await;
            }
        }
    }
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
    last_dht_refresh: Instant,
    dht_startup_refreshes: usize,
}

impl RuntimeState {
    fn new(
        cfg: &NodeConfig,
        resolved_config: &ResolvedNodeConfig,
        relay_reservation_plan: RelayReservationPlan,
        relay_selection_plan: RelaySelectionPlan,
        rendezvous_state: RendezvousState,
        dht_state: DhtProviderState,
        peer_book: PeerBook,
    ) -> Self {
        Self {
            rep: ReputationStore::new(cfg.message_security.reputation.clone()),
            replay_cache: HeartbeatReplayCache::new(&cfg.message_security),
            relay_state: runtime_maintenance::initial_relay_state(
                cfg,
                resolved_config,
                &relay_reservation_plan,
                &relay_selection_plan,
            ),
            rendezvous_state,
            dht_state,
            peer_book,
            pending_connections: PendingConnectionPlans::default(),
            auto_dial_stats: AutoDialStats::default(),
            connection_caps: ConnectionCapState::new(&cfg.connection_limits),
            app_topic_hashes: Vec::new(),
            metrics: NodeMetrics::default(),
            identify_addresses: IdentifyAddressState::default(),
            last_dht_refresh: Instant::now(),
            dht_startup_refreshes: 0,
        }
    }
}

async fn tick_runtime(
    cfg: &NodeConfig,
    swarm: &mut Swarm<MeshBehaviour>,
    local_peer: PeerId,
    heartbeat_topic: &IdentTopic,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    runtime_state: &mut RuntimeState,
    started_at: std::time::Instant,
) {
    events::enforce_relay_schedule(&cfg.relay, swarm, snapshot, &mut runtime_state.relay_state)
        .await;
    runtime_maintenance::close_expired_unverified_relayed(
        swarm,
        snapshot,
        &mut runtime_state.relay_state,
    )
    .await;
    runtime_maintenance::maintain_application_connections(
        cfg,
        swarm,
        local_peer,
        snapshot,
        &runtime_state.peer_book,
        &mut runtime_state.pending_connections,
        &mut runtime_state.dht_state,
        &mut runtime_state.auto_dial_stats,
    )
    .await;
    if let Ok(bytes) = publish_heartbeat(swarm, local_peer, heartbeat_topic, snapshot).await {
        runtime_state
            .metrics
            .bandwidth
            .record_sent(None, Some("heartbeat"), bytes);
    }
    runtime_state.metrics.compute.execution_cycles_estimated = runtime_state
        .metrics
        .compute
        .execution_cycles_estimated
        .saturating_add(1);
    runtime_state.metrics.compute.active_request_count =
        u32::try_from(runtime_state.pending_connections.pending_count()).unwrap_or(u32::MAX);
    let dht_refresh_secs = if runtime_state.dht_startup_refreshes < DHT_STARTUP_REFRESHES {
        cfg.discovery
            .dht
            .refresh_interval_secs
            .clamp(1, DHT_STARTUP_REFRESH_INTERVAL_SECS)
    } else {
        cfg.discovery.dht.refresh_interval_secs.max(1)
    };
    let dht_refresh_interval = Duration::from_secs(dht_refresh_secs);
    let dht_plan = if runtime_state.last_dht_refresh.elapsed() >= dht_refresh_interval {
        runtime_state.last_dht_refresh = Instant::now();
        runtime_state.dht_startup_refreshes = runtime_state.dht_startup_refreshes.saturating_add(1);
        Some(start_dht_namespace_discovery_with_interval(
            swarm,
            cfg.network_id,
            &cfg.discovery,
            runtime_state.rendezvous_state.registered_with.len(),
            &mut runtime_state.dht_state,
            dht_refresh_secs,
        ))
    } else {
        None
    };

    let mut guard = snapshot.lock().await;
    runtime_state.metrics.uptime_seconds = started_at.elapsed().as_secs();
    runtime_state.metrics.compute.choked_peers_count =
        u32::try_from(guard.connection_cap_disconnects).unwrap_or(u32::MAX);
    guard.uptime_secs = runtime_state.metrics.uptime_seconds;
    if let Some(plan) = dht_plan {
        apply_dht_refresh_snapshot(&mut guard, &runtime_state.dht_state, &plan, "periodic");
    }
}
