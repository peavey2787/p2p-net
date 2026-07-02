//! Long-running node runtime loop.
//!
//! Startup code constructs the swarm and initial discovery state, then hands
//! ownership to this module so `node::mod` stays focused on public startup.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::{IdentTopic, TopicHash};
use libp2p::{Multiaddr, PeerId, Swarm};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::api::AppMessage;
use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::{start_dht_namespace_discovery, DhtProviderState};
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::relay::{RelayReservationPlan, RelayState};
use crate::connectivity::relay_discovery::RelaySelectionPlan;
use crate::connectivity::rendezvous::RendezvousState;
use crate::platform::NodeStorage;
use crate::protocol::pulse::HeartbeatReplayCache;
use crate::protocol::reputation::ReputationStore;
use crate::stack::MeshBehaviour;

use super::commands::{self, NodeCommandContext};
use super::dial::AutoDialStats;
use super::events::{self, SwarmEventContext};
use super::handle::NodeCommand;
use super::profile::ResolvedNodeConfig;
use super::public_ip;
use super::push_pulse;
use super::runtime_tasks::{
    apply_dht_refresh_snapshot, apply_public_ip_probe_result, publish_heartbeat,
};
use super::config::NodeConfig;
use super::snapshot::NodeSnapshot;

const DHT_REFRESH_TICKS: usize = 1;

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
                            dcutr_policy: &cfg.dcutr,
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
    dht_refresh_ticks: usize,
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
            relay_state: initial_relay_state(
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
            dht_refresh_ticks: 0,
        }
    }
}

fn initial_relay_state(
    cfg: &NodeConfig,
    resolved_config: &ResolvedNodeConfig,
    relay_reservation_plan: &RelayReservationPlan,
    relay_selection_plan: &RelaySelectionPlan,
) -> RelayState {
    RelayState {
        server_enabled: cfg.relay.is_active_now(),
        health: cfg.relay.health_now(),
        relay_client_reservation_attempts: relay_reservation_plan.attempted,
        relay_client_reservation_failures: relay_reservation_plan.errors.len(),
        dcutr_enabled: resolved_config.dcutr_enabled,
        relay_discovery_selected_relays: relay_selection_plan
            .selected_strings()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        relay_discovery_candidate_count: relay_selection_plan.total_candidates(),
        relay_discovery_configured_candidates: relay_selection_plan.configured_candidates,
        relay_discovery_cached_candidates: relay_selection_plan.cached_candidates,
        relay_discovery_rendezvous_candidates: relay_selection_plan.rendezvous_candidates,
        relay_discovery_public_candidates: relay_selection_plan.public_candidates,
        relay_discovery_ignored_candidates: relay_selection_plan.ignored_candidates,
        relay_discovery_failures: relay_selection_plan
            .errors
            .len()
            .saturating_add(relay_reservation_plan.errors.len()),
        relay_discovery_replacements: 0,
        relayed_listen_addrs: relay_reservation_plan
            .listen_addrs
            .iter()
            .map(ToString::to_string)
            .collect(),
        ..RelayState::default()
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
    let _ = publish_heartbeat(swarm, local_peer, heartbeat_topic, snapshot).await;
    runtime_state.dht_refresh_ticks = runtime_state.dht_refresh_ticks.saturating_add(1);
    let dht_plan = if runtime_state.dht_refresh_ticks >= DHT_REFRESH_TICKS {
        runtime_state.dht_refresh_ticks = 0;
        Some(start_dht_namespace_discovery(
            swarm,
            cfg.network_id,
            &cfg.discovery,
            runtime_state.rendezvous_state.registered_with.len(),
            &mut runtime_state.dht_state,
        ))
    } else {
        None
    };

    let mut guard = snapshot.lock().await;
    guard.uptime_secs = started_at.elapsed().as_secs();
    if let Some(plan) = dht_plan {
        apply_dht_refresh_snapshot(&mut guard, &runtime_state.dht_state, &plan, "periodic");
    }
}
