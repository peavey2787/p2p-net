//! Standalone node: libp2p swarm + heartbeat loop, split into `types` and `events`.

mod events;
mod profile;
mod types;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::IdentTopic;
use libp2p::{PeerId, Swarm};
use serde_json::{Map, Value};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::common::error::NetError;
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::relay::RelayState;
use crate::connectivity::rendezvous::RendezvousState;
use crate::connectivity::{dns, identity, peer_cache};
use crate::protocol::pulse::{collect_local_heartbeat, heartbeat_topic, HeartbeatReplayCache};
use crate::protocol::reputation::ReputationStore;
use crate::stack::{
    build_swarm, refresh_rendezvous, reserve_configured_relays, seed_bootstrap,
    startup_discovery_plan, MeshBehaviour,
};

pub use profile::{BehaviourSet, NodeProfile, NodeRole, ResolvedNodeConfig};
pub use types::{NodeConfig, NodeSnapshot};

use types::network_label;

#[derive(Clone)]
pub struct NodeHandle {
    pub peer_id: PeerId,
    pub snapshot: Arc<Mutex<NodeSnapshot>>,
    shutdown_tx: mpsc::Sender<()>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl NodeHandle {
    /// Request shutdown and wait for the swarm task to exit.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}

pub async fn start_node(cfg: NodeConfig) -> Result<NodeHandle, NetError> {
    cfg.validate()?;
    let cfg = cfg.with_profile_defaults_applied();
    cfg.validate()?;
    let local_key = identity::load_or_create_identity_key(&cfg.identity_key_path)?;
    let local_peer = PeerId::from(local_key.public());
    let (mut swarm, transport_plan) = build_swarm(local_key, &cfg).await?;

    let heartbeat_topic = IdentTopic::new(heartbeat_topic(cfg.network_id));
    let heartbeat_topic_hash = heartbeat_topic.hash().clone();
    let _ = swarm.behaviour_mut().gossipsub.subscribe(&heartbeat_topic);

    let bootstrap_peers = dns::resolve_configured_multiaddrs(
        "bootstrap_peers",
        cfg.parsed_bootstrap_peers()?,
        &cfg.dnsaddr,
    )
    .await?;
    let bootstrap_seed_peers = dns::resolve_configured_multiaddrs(
        "discovery.bootstrap_seed_peers",
        cfg.parsed_bootstrap_seed_peers()?,
        &cfg.dnsaddr,
    )
    .await?;
    let rendezvous_peers = dns::resolve_configured_multiaddrs(
        "discovery.rendezvous_peers",
        cfg.parsed_rendezvous_peers()?,
        &cfg.dnsaddr,
    )
    .await?;
    let relay_peers =
        dns::resolve_configured_multiaddrs("relay_peers", cfg.parsed_relay_peers()?, &cfg.dnsaddr)
            .await?;
    let cached_peers = dns::resolve_cached_multiaddrs(
        peer_cache::load_last_addrs(&cfg.discovery, cfg.startup_peer_cache_probe),
        &cfg.dnsaddr,
    )
    .await;

    let startup_plan = startup_discovery_plan(
        bootstrap_peers,
        bootstrap_seed_peers,
        rendezvous_peers.clone(),
        cached_peers,
    );
    seed_bootstrap(&mut swarm, &startup_plan.dial_addrs);

    let relay_reservation_plan = if cfg.reserve_configured_relays {
        reserve_configured_relays(&mut swarm, &relay_peers)
    } else {
        seed_bootstrap(&mut swarm, &relay_peers);
        Default::default()
    };

    let mut rendezvous_state = RendezvousState::default();
    let rendezvous_plan = refresh_rendezvous(
        &mut swarm,
        &cfg.discovery,
        &rendezvous_peers,
        &mut rendezvous_state,
    );

    let snapshot = Arc::new(Mutex::new(NodeSnapshot {
        network_id: cfg.network_id,
        network_label: network_label(cfg.network_id),
        peer_id: local_peer.to_string(),
        nat_status: "unknown".to_string(),
        public_addr: None,
        active_transports: transport_plan
            .active
            .iter()
            .map(|s| s.to_string())
            .collect(),
        connected_peers: 0,
        relay_server_enabled: cfg.relay.is_active_now(),
        relay_service_health: cfg.relay.health_now(),
        relay_acl_scope: "connection_level".to_string(),
        relay_reservations_accepted: 0,
        relay_active_circuits: 0,
        relay_denied_requests: 0,
        relay_bytes_forwarded: 0,
        relay_denied_reservations: 0,
        relay_denied_circuits: 0,
        relay_rate_limited_events: 0,
        relay_at_capacity_events: 0,
        relay_server_errors: 0,
        connection_limit_events: 0,
        connection_cap_disconnects: 0,
        relay_client_reservations: 0,
        relay_client_reservation_attempts: relay_reservation_plan.attempted,
        relay_client_reservation_failures: relay_reservation_plan.errors.len(),
        relayed_listen_addresses: relay_reservation_plan
            .listen_addrs
            .iter()
            .map(ToString::to_string)
            .collect(),
        dcutr_attempts: 0,
        dcutr_successes: 0,
        rendezvous_client_enabled: cfg.discovery.rendezvous.client_enabled,
        rendezvous_server_enabled: cfg.discovery.rendezvous.server_enabled,
        rendezvous_registered_with: rendezvous_state.registered_with.len(),
        rendezvous_discovered_peers: rendezvous_state.discovered_peers.len(),
        rendezvous_register_attempts: rendezvous_state.register_attempts,
        rendezvous_register_failures: rendezvous_state.register_failures,
        rendezvous_discover_attempts: rendezvous_state.discover_attempts,
        rendezvous_discover_failures: rendezvous_state.discover_failures,
        rendezvous_server_registrations: rendezvous_state.server_registrations,
        rendezvous_server_discoveries_served: rendezvous_state.server_discoveries_served,
        rendezvous_server_errors: rendezvous_state.server_errors,
        gossip_messages_rejected: 0,
        gossip_messages_ignored: 0,
        gossip_messages_accepted: 0,
        pulses: VecDeque::new(),
        uptime_secs: 0,
    }));

    if !relay_reservation_plan.listen_addrs.is_empty() || !relay_reservation_plan.errors.is_empty()
    {
        let mut guard = snapshot.lock().await;
        for addr in &relay_reservation_plan.listen_addrs {
            push_pulse(
                &mut guard.pulses,
                format!("relay_client reservation requested via {addr}"),
            );
        }
        for err in &relay_reservation_plan.errors {
            push_pulse(
                &mut guard.pulses,
                format!("relay_client reservation setup error: {err}"),
            );
        }
    }

    if rendezvous_plan.register_attempts > 0
        || rendezvous_plan.discover_attempts > 0
        || !rendezvous_plan.errors.is_empty()
    {
        let mut guard = snapshot.lock().await;
        for err in &rendezvous_plan.errors {
            push_pulse(
                &mut guard.pulses,
                format!("rendezvous startup error: {err}"),
            );
        }
        if rendezvous_plan.register_attempts > 0 || rendezvous_plan.discover_attempts > 0 {
            push_pulse(
                &mut guard.pulses,
                format!(
                    "rendezvous startup register_attempts={} discover_attempts={}",
                    rendezvous_plan.register_attempts, rendezvous_plan.discover_attempts
                ),
            );
        }
    }

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    let task_snapshot = Arc::clone(&snapshot);
    let rendezvous_peers_for_task = rendezvous_peers.clone();
    let task = tokio::spawn(async move {
        let heartbeat_interval = Duration::from_secs(cfg.heartbeat_interval_secs.max(1));
        let mut ticker = tokio::time::interval(heartbeat_interval);
        let mut rep = ReputationStore::new(cfg.message_security.reputation.clone());
        let mut replay_cache = HeartbeatReplayCache::new(&cfg.message_security);
        let mut relay_state = RelayState {
            server_enabled: cfg.relay.is_active_now(),
            health: cfg.relay.health_now(),
            relay_client_reservation_attempts: relay_reservation_plan.attempted,
            relay_client_reservation_failures: relay_reservation_plan.errors.len(),
            relayed_listen_addrs: relay_reservation_plan
                .listen_addrs
                .iter()
                .map(ToString::to_string)
                .collect(),
            ..RelayState::default()
        };
        let mut rendezvous_state = rendezvous_state;
        let mut connection_caps = ConnectionCapState::new(&cfg.connection_limits);
        let started_at = std::time::Instant::now();

        loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    events::enforce_relay_schedule(
                        &cfg.relay,
                        &mut swarm,
                        &task_snapshot,
                        &mut relay_state,
                    ).await;
                    let _ = publish_heartbeat(&mut swarm, local_peer, &heartbeat_topic, &task_snapshot).await;
                    let mut guard = task_snapshot.lock().await;
                    guard.uptime_secs = started_at.elapsed().as_secs();
                }
                maybe_shutdown = shutdown_rx.recv() => {
                    let _ = maybe_shutdown;
                    break;
                }
                evt = swarm.select_next_some() => {
                    let mut event_ctx = events::SwarmEventContext {
                        snapshot: &task_snapshot,
                        rep: &mut rep,
                        relay_state: &mut relay_state,
                        rendezvous_state: &mut rendezvous_state,
                        connection_caps: &mut connection_caps,
                        relay_cfg: &cfg.relay,
                        discovery_cfg: &cfg.discovery,
                        rendezvous_peers: &rendezvous_peers_for_task,
                        message_security: &cfg.message_security,
                        replay_cache: &mut replay_cache,
                        heartbeat_topic_hash: &heartbeat_topic_hash,
                    };
                    events::handle_swarm_event(evt, &mut swarm, &mut event_ctx).await;
                }
            }
        }
    });

    Ok(NodeHandle {
        peer_id: local_peer,
        snapshot,
        shutdown_tx,
        task: Arc::new(Mutex::new(Some(task))),
    })
}

async fn publish_heartbeat(
    swarm: &mut Swarm<MeshBehaviour>,
    local_peer: PeerId,
    topic: &IdentTopic,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
) -> Result<(), NetError> {
    let env = collect_local_heartbeat(local_peer)?;
    let payload = serde_json::to_vec(&env).map_err(|e| NetError::GossipCodec(e.to_string()))?;
    let _ = swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.clone(), payload);
    let mut guard = snapshot.lock().await;
    push_pulse(
        &mut guard.pulses,
        format!("local heartbeat {} {}", env.peer_id, env.nonce_hex),
    );
    Ok(())
}

pub(crate) fn push_pulse(buf: &mut VecDeque<String>, line: String) {
    tracing::info!(target: "p2p_net::event", event = %line);
    buf.push_front(line);
    while buf.len() > 24 {
        let _ = buf.pop_back();
    }
}

pub fn snapshot_to_json(snapshot: &NodeSnapshot) -> Value {
    fn insert<T: serde::Serialize>(map: &mut Map<String, Value>, key: &str, value: T) {
        let value = serde_json::to_value(value).unwrap_or(Value::Null);
        map.insert(key.to_owned(), value);
    }

    let mut map = Map::new();
    insert(&mut map, "peer_id", &snapshot.peer_id);
    insert(&mut map, "network_id", snapshot.network_id);
    insert(&mut map, "network_label", &snapshot.network_label);
    insert(&mut map, "nat_status", &snapshot.nat_status);
    insert(&mut map, "public_addr", &snapshot.public_addr);
    insert(&mut map, "active_transports", &snapshot.active_transports);
    insert(&mut map, "connected_peers", snapshot.connected_peers);
    insert(
        &mut map,
        "relay_server_enabled",
        snapshot.relay_server_enabled,
    );
    insert(
        &mut map,
        "relay_service_health",
        snapshot.relay_service_health.as_str(),
    );
    insert(&mut map, "relay_acl_scope", &snapshot.relay_acl_scope);
    insert(
        &mut map,
        "relay_reservations_accepted",
        snapshot.relay_reservations_accepted,
    );
    insert(
        &mut map,
        "relay_client_reservations",
        snapshot.relay_client_reservations,
    );
    insert(
        &mut map,
        "relay_active_circuits",
        snapshot.relay_active_circuits,
    );
    insert(
        &mut map,
        "relay_denied_requests",
        snapshot.relay_denied_requests,
    );
    insert(
        &mut map,
        "relay_bytes_forwarded",
        snapshot.relay_bytes_forwarded,
    );
    insert(
        &mut map,
        "relay_denied_reservations",
        snapshot.relay_denied_reservations,
    );
    insert(
        &mut map,
        "relay_denied_circuits",
        snapshot.relay_denied_circuits,
    );
    insert(
        &mut map,
        "relay_rate_limited_events",
        snapshot.relay_rate_limited_events,
    );
    insert(
        &mut map,
        "relay_at_capacity_events",
        snapshot.relay_at_capacity_events,
    );
    insert(
        &mut map,
        "relay_server_errors",
        snapshot.relay_server_errors,
    );
    insert(
        &mut map,
        "connection_limit_events",
        snapshot.connection_limit_events,
    );
    insert(
        &mut map,
        "connection_cap_disconnects",
        snapshot.connection_cap_disconnects,
    );
    insert(
        &mut map,
        "relay_client_reservation_attempts",
        snapshot.relay_client_reservation_attempts,
    );
    insert(
        &mut map,
        "relay_client_reservation_failures",
        snapshot.relay_client_reservation_failures,
    );
    insert(
        &mut map,
        "relayed_listen_addresses",
        &snapshot.relayed_listen_addresses,
    );
    insert(&mut map, "dcutr_attempts", snapshot.dcutr_attempts);
    insert(&mut map, "dcutr_successes", snapshot.dcutr_successes);
    insert(
        &mut map,
        "rendezvous_client_enabled",
        snapshot.rendezvous_client_enabled,
    );
    insert(
        &mut map,
        "rendezvous_server_enabled",
        snapshot.rendezvous_server_enabled,
    );
    insert(
        &mut map,
        "rendezvous_registered_with",
        snapshot.rendezvous_registered_with,
    );
    insert(
        &mut map,
        "rendezvous_discovered_peers",
        snapshot.rendezvous_discovered_peers,
    );
    insert(
        &mut map,
        "rendezvous_register_attempts",
        snapshot.rendezvous_register_attempts,
    );
    insert(
        &mut map,
        "rendezvous_register_failures",
        snapshot.rendezvous_register_failures,
    );
    insert(
        &mut map,
        "rendezvous_discover_attempts",
        snapshot.rendezvous_discover_attempts,
    );
    insert(
        &mut map,
        "rendezvous_discover_failures",
        snapshot.rendezvous_discover_failures,
    );
    insert(
        &mut map,
        "rendezvous_server_registrations",
        snapshot.rendezvous_server_registrations,
    );
    insert(
        &mut map,
        "rendezvous_server_discoveries_served",
        snapshot.rendezvous_server_discoveries_served,
    );
    insert(
        &mut map,
        "rendezvous_server_errors",
        snapshot.rendezvous_server_errors,
    );
    insert(
        &mut map,
        "gossip_messages_rejected",
        snapshot.gossip_messages_rejected,
    );
    insert(
        &mut map,
        "gossip_messages_ignored",
        snapshot.gossip_messages_ignored,
    );
    insert(
        &mut map,
        "gossip_messages_accepted",
        snapshot.gossip_messages_accepted,
    );
    insert(&mut map, "pulses", &snapshot.pulses);
    insert(&mut map, "uptime_secs", snapshot.uptime_secs);

    Value::Object(map)
}

/// Export operator counters in Prometheus text exposition format without opening an HTTP port.
/// Embedders that want an HTTP endpoint can serve this string from their own trusted admin server.
pub fn snapshot_to_prometheus_metrics(snapshot: &NodeSnapshot) -> String {
    fn line(name: &str, value: impl std::fmt::Display, out: &mut String) {
        out.push_str(name);
        out.push(' ');
        out.push_str(&value.to_string());
        out.push('\n');
    }

    let mut out = String::new();
    line("p2p_connected_peers", snapshot.connected_peers, &mut out);
    line(
        "p2p_relay_server_enabled",
        if snapshot.relay_server_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_relay_reservations_accepted",
        snapshot.relay_reservations_accepted,
        &mut out,
    );
    line(
        "p2p_relay_client_reservations",
        snapshot.relay_client_reservations,
        &mut out,
    );
    line(
        "p2p_relay_active_circuits",
        snapshot.relay_active_circuits,
        &mut out,
    );
    line(
        "p2p_relay_denied_requests",
        snapshot.relay_denied_requests,
        &mut out,
    );
    line(
        "p2p_relay_bytes_forwarded",
        snapshot.relay_bytes_forwarded,
        &mut out,
    );
    line("p2p_dcutr_attempts", snapshot.dcutr_attempts, &mut out);
    line("p2p_dcutr_successes", snapshot.dcutr_successes, &mut out);
    line(
        "p2p_gossip_messages_accepted",
        snapshot.gossip_messages_accepted,
        &mut out,
    );
    line(
        "p2p_gossip_messages_ignored",
        snapshot.gossip_messages_ignored,
        &mut out,
    );
    line(
        "p2p_gossip_messages_rejected",
        snapshot.gossip_messages_rejected,
        &mut out,
    );
    out
}
