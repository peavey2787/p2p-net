use std::sync::Arc;

use libp2p::gossipsub::IdentTopic;
use libp2p::{PeerId, Swarm};
use tokio::sync::Mutex;

use crate::connectivity::dht::start_dht_namespace_discovery_with_interval;
use crate::platform::NodeStorage;
use crate::stack::MeshBehaviour;

use super::super::config::NodeConfig;
use super::super::events;
use super::super::runtime_maintenance;
use super::super::runtime_tasks::{apply_dht_refresh_snapshot, publish_heartbeat};
use super::super::snapshot::NodeSnapshot;
use super::{RuntimeState, PEER_CACHE_FLUSH_INTERVAL};

pub(super) async fn tick_runtime(
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

    runtime_state.rep.tick_decay();

    let mut pulses = Vec::new();
    if let Some(pulse) =
        runtime_maintenance::close_expired_unverified_relayed(swarm, &mut runtime_state.relay_state)
    {
        pulses.push(pulse);
    }
    pulses.extend(runtime_maintenance::maintain_application_connections(
        cfg,
        swarm,
        local_peer,
        &runtime_state.peer_book,
        &mut runtime_state.pending_connections,
        &mut runtime_state.dht_state,
        &mut runtime_state.auto_dial_stats,
    ));
    if let Ok(published) = publish_heartbeat(swarm, local_peer, heartbeat_topic) {
        runtime_state.metrics.bandwidth.record_sent(
            None,
            Some("heartbeat"),
            published.accounted_bytes,
        );
        pulses.push(published.pulse);
    }

    runtime_state.metrics.compute.execution_cycles_estimated = runtime_state
        .metrics
        .compute
        .execution_cycles_estimated
        .saturating_add(1);
    runtime_state.metrics.compute.active_request_count =
        u32::try_from(runtime_state.pending_connections.pending_count()).unwrap_or(u32::MAX);
    runtime_state.metrics.uptime_seconds = started_at.elapsed().as_secs();

    let mut guard = snapshot.lock().await;
    runtime_state.metrics.compute.choked_peers_count =
        u32::try_from(guard.connection_cap_disconnects).unwrap_or(u32::MAX);
    guard.uptime_secs = runtime_state.metrics.uptime_seconds;
    guard.apply_relay_state(&runtime_state.relay_state);
    events::sync_peer_connectivity_fields(
        &mut guard,
        &runtime_state.peer_book,
        &runtime_state.auto_dial_stats,
        &runtime_state.pending_connections,
        cfg.discovery.public_bootstrap.auto_connect_discovered_peers,
    );
    for pulse in pulses {
        super::super::push_pulse(&mut guard.pulses, pulse);
    }
}

pub(super) async fn flush_observability(
    cfg: &NodeConfig,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    storage: &dyn NodeStorage,
    runtime_state: &mut RuntimeState,
) -> bool {
    if runtime_state.last_peer_cache_flush.elapsed() >= PEER_CACHE_FLUSH_INTERVAL {
        runtime_state.flush_peer_cache(cfg, storage);
    }
    if runtime_state.observability.is_empty() {
        return false;
    }
    let mut guard = snapshot.lock().await;
    events::flush_observability_snapshot(
        &mut guard,
        &mut runtime_state.observability,
        &runtime_state.dht_state,
        &runtime_state.peer_book,
        &runtime_state.auto_dial_stats,
        &runtime_state.pending_connections,
        cfg.discovery.public_bootstrap.auto_connect_discovered_peers,
    );
    true
}

pub(super) async fn refresh_dht(
    cfg: &NodeConfig,
    swarm: &mut Swarm<MeshBehaviour>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    runtime_state: &mut RuntimeState,
    reason: &str,
) {
    let refresh_interval_secs = runtime_state.dht_refresh_schedule.current_interval_secs();
    let plan = start_dht_namespace_discovery_with_interval(
        swarm,
        cfg.network_id,
        &cfg.discovery,
        runtime_state.rendezvous_state.registered_with.len(),
        &mut runtime_state.dht_state,
        refresh_interval_secs,
    );
    runtime_state.dht_refresh_schedule.record_refresh();

    let mut guard = snapshot.lock().await;
    apply_dht_refresh_snapshot(&mut guard, &runtime_state.dht_state, &plan, reason);
}
