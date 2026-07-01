//! Standalone node orchestration: libp2p swarm ownership, command routing, events, and snapshots.

mod capabilities;
mod commands;
mod environment;
mod events;
mod handle;
mod profile;
mod types;

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::IdentTopic;
use libp2p::{Multiaddr, PeerId, Swarm};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::api::PeerSource;
use crate::common::error::NetError;
use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::{start_dht_namespace_discovery, DhtProviderState};
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::relay::RelayState;
use crate::connectivity::rendezvous::RendezvousState;
use crate::connectivity::{dns, identity, peer_cache, relay_discovery};
use crate::platform::{DesktopPlatformRuntime, NodeStorage, PlatformRuntime};
use crate::protocol::pulse::{collect_local_heartbeat, heartbeat_topic, HeartbeatReplayCache};
use crate::protocol::reputation::ReputationStore;
use crate::stack::{
    build_swarm, refresh_rendezvous, reserve_configured_relays, seed_bootstrap,
    extract_p2p_peer_id, startup_discovery_plan, startup_discovery_plan_with_public, MeshBehaviour,
};

pub use capabilities::{apply_resolved_capabilities, resolve_node_config};
pub use environment::{
    EnvironmentConfig, EnvironmentReport, NatKind, NetworkReachability, PlatformKind,
};
pub use profile::{BehaviourSet, NodeProfile, NodeRole, ResolvedNodeConfig};
pub use handle::NodeHandle;
pub use types::{NodeConfig, NodeSnapshot};

use types::network_label;

pub async fn start_node(cfg: NodeConfig) -> Result<NodeHandle, NetError> {
    let desktop = Arc::new(DesktopPlatformRuntime::default());
    let runtime: Arc<dyn PlatformRuntime> = desktop.clone();
    let storage: Arc<dyn NodeStorage> = desktop;
    start_node_with_platform(cfg, runtime, storage).await
}

/// Start a node with platform-supplied runtime hints and storage. This keeps the
/// P2P core shared while allowing Android/iOS/Desktop shells to adapt storage,
/// data directories, and lifecycle restrictions without separate node logic.
pub async fn start_node_with_platform(
    cfg: NodeConfig,
    platform_runtime: Arc<dyn PlatformRuntime>,
    storage: Arc<dyn NodeStorage>,
) -> Result<NodeHandle, NetError> {
    cfg.validate()?;
    let environment_report = cfg.environment_report_with_runtime(platform_runtime.as_ref());
    let resolved_config = cfg.try_resolved_for_environment(&environment_report)?;
    let cfg = cfg.with_resolved_capabilities_applied(&resolved_config);
    cfg.validate()?;
    let local_key = identity::load_or_create_identity_key_with_storage(
        &cfg.identity_key_path,
        storage.as_ref(),
    )?;
    let local_peer = PeerId::from(local_key.public());
    let (mut swarm, transport_plan) = build_swarm(local_key, &cfg, &resolved_config).await?;

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
    let public_bootstrap_seed_peers = dns::resolve_configured_multiaddrs(
        "discovery.public_bootstrap.bootstrap_seed_peers",
        cfg.parsed_public_bootstrap_seed_peers()?,
        &cfg.dnsaddr,
    )
    .await?;
    let public_relay_peers = dns::resolve_configured_multiaddrs(
        "discovery.public_bootstrap.relay_peers",
        cfg.parsed_public_relay_peers()?,
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
    let cached_startup_addrs = peer_cache::load_last_addrs_with_storage(
        &cfg.discovery,
        cfg.startup_peer_cache_probe
            .max(cfg.discovery.relay_discovery.max_reservations),
        storage.as_ref(),
    );
    let cached_peers =
        dns::resolve_cached_multiaddrs(cached_startup_addrs.clone(), &cfg.dnsaddr).await;
    let cached_relay_peers =
        dns::resolve_cached_multiaddrs(cached_startup_addrs, &cfg.dnsaddr).await;

    let owned_startup_candidate_count = startup_discovery_plan(
        bootstrap_peers.clone(),
        bootstrap_seed_peers.clone(),
        rendezvous_peers.clone(),
        cached_peers.clone(),
    )
    .dial_addrs
    .len();
    let public_bootstrap_decision = cfg
        .discovery
        .public_bootstrap
        .bootstrap_decision(owned_startup_candidate_count);
    let startup_plan = startup_discovery_plan_with_public(
        bootstrap_peers.clone(),
        bootstrap_seed_peers.clone(),
        rendezvous_peers.clone(),
        cached_peers.clone(),
        if public_bootstrap_decision.used {
            public_bootstrap_seed_peers.clone()
        } else {
            Vec::new()
        },
        public_bootstrap_decision.used,
    );
    seed_bootstrap(&mut swarm, &startup_plan.dial_addrs);

    let mut peer_book = PeerBook::default();
    record_peer_book_addrs(&mut peer_book, &bootstrap_peers, PeerSource::Bootstrap);
    record_peer_book_addrs(&mut peer_book, &bootstrap_seed_peers, PeerSource::BootstrapSeed);
    record_peer_book_addrs(&mut peer_book, &rendezvous_peers, PeerSource::Rendezvous);
    record_peer_book_addrs(&mut peer_book, &cached_peers, PeerSource::PeerCache);
    if public_bootstrap_decision.used {
        record_peer_book_addrs(
            &mut peer_book,
            &public_bootstrap_seed_peers,
            PeerSource::BootstrapSeed,
        );
    }

    let relay_discovery_policy = if resolved_config.relay_discovery_enabled {
        cfg.discovery.relay_discovery.clone()
    } else {
        relay_discovery::RelayDiscoveryPolicy {
            enabled: false,
            ..cfg.discovery.relay_discovery.clone()
        }
    };
    let configured_relay_peers = relay_peers;
    let startup_cached_relay_peers = cached_relay_peers;
    let startup_rendezvous_relay_peers = rendezvous_peers.clone();
    let owned_relay_selection_plan = relay_discovery::select_startup_relays(
        &relay_discovery_policy,
        configured_relay_peers.clone(),
        startup_cached_relay_peers.clone(),
        startup_rendezvous_relay_peers.clone(),
        Vec::new(),
    );
    let public_relay_decision = cfg
        .discovery
        .public_bootstrap
        .relay_decision(owned_relay_selection_plan.selected_addrs.len());
    let relay_selection_plan = if public_relay_decision.used {
        relay_discovery::select_startup_relays(
            &relay_discovery_policy,
            configured_relay_peers,
            startup_cached_relay_peers,
            startup_rendezvous_relay_peers,
            public_relay_peers,
        )
    } else {
        owned_relay_selection_plan
    };

    let selected_relay_peers = relay_selection_plan.selected_addrs.clone();
    record_peer_book_addrs(&mut peer_book, &selected_relay_peers, PeerSource::RelayDiscovery);
    let relay_reservation_plan = if resolved_config.enabled_behaviours.relay_client
        && cfg.reserve_configured_relays
    {
        reserve_configured_relays(&mut swarm, &selected_relay_peers)
    } else {
        if resolved_config.enabled_behaviours.relay_client && !selected_relay_peers.is_empty() {
            seed_bootstrap(&mut swarm, &selected_relay_peers);
        }
        Default::default()
    };

    let discovery_namespaces = cfg
        .discovery
        .rendezvous_namespaces(cfg.network_id)
        .unwrap_or_else(|_| vec![cfg.discovery.rendezvous.namespace.clone()]);
    let discovery_namespace_mode = if cfg.discovery.namespace.is_enabled() {
        cfg.discovery.namespace.privacy.as_str().to_string()
    } else {
        "operator".to_string()
    };

    let mut active_transports: Vec<String> = transport_plan
        .active
        .iter()
        .map(|s| s.to_string())
        .collect();
    if public_bootstrap_decision.used {
        active_transports.push("public-bootstrap-fallback".to_string());
    }
    if public_relay_decision.used {
        active_transports.push("public-relay-fallback".to_string());
    }

    let mut rendezvous_state = RendezvousState::default();
    let rendezvous_plan = refresh_rendezvous(
        &mut swarm,
        cfg.network_id,
        &cfg.discovery,
        &rendezvous_peers,
        &mut rendezvous_state,
    );

    let mut dht_state = DhtProviderState::default();
    let dht_plan = start_dht_namespace_discovery(
        &mut swarm,
        cfg.network_id,
        &cfg.discovery,
        rendezvous_peers.len(),
        &mut dht_state,
    );

    let snapshot = Arc::new(Mutex::new(NodeSnapshot {
        network_id: cfg.network_id,
        network_label: network_label(cfg.network_id),
        peer_id: local_peer.to_string(),
        nat_status: "unknown".to_string(),
        public_addr: None,
        environment_platform: environment_report.platform.as_str().to_string(),
        environment_reachability: environment_report.reachability.as_str().to_string(),
        environment_nat_status: environment_report.nat_status.as_str().to_string(),
        environment_can_accept_inbound: environment_report.can_accept_inbound,
        environment_likely_cgnat: environment_report.likely_cgnat,
        environment_battery_sensitive: environment_report.battery_sensitive,
        environment_background_restricted: environment_report.background_restricted,
        platform_runtime: platform_runtime.runtime_name().to_string(),
        platform_storage: storage.storage_kind().to_string(),
        platform_default_data_dir: platform_runtime
            .default_data_dir()
            .map(|path| path.display().to_string()),
        platform_can_listen_tcp: platform_runtime.can_listen_tcp(),
        platform_can_listen_quic: platform_runtime.can_listen_quic(),
        active_transports,
        discovery_namespace_mode,
        discovery_namespace_count: discovery_namespaces.len(),
        discovery_namespaces,
        dht_provider_enabled: cfg.discovery.dht.enabled,
        dht_provider_announce_enabled: cfg.discovery.dht.announce,
        dht_provider_discover_enabled: cfg.discovery.dht.discover,
        dht_provider_namespaces_announced: dht_state.namespaces_announced.len(),
        dht_provider_announce_attempts: dht_state.announce_attempts,
        dht_provider_announce_failures: dht_state.announce_failures,
        dht_provider_queries: dht_state.provider_queries,
        dht_provider_query_failures: dht_state.provider_query_failures,
        dht_provider_records_found: dht_state.provider_records_found,
        dht_provider_queries_finished: dht_state.provider_queries_finished,
        dht_provider_peers_discovered: dht_state.provider_peer_count(),
        public_fallback_mode: cfg.discovery.public_bootstrap.mode.as_str().to_string(),
        public_fallback_used: public_bootstrap_decision.used || public_relay_decision.used,
        public_fallback_reason: if public_bootstrap_decision.used {
            public_bootstrap_decision.reason.clone()
        } else if public_relay_decision.used {
            public_relay_decision.reason.clone()
        } else {
            "not_used".to_string()
        },
        public_bootstrap_seed_count: startup_plan.public_bootstrap_seed_count,
        public_relay_candidate_count: relay_selection_plan.public_candidates,
        connected_peers: 0,
        peer_book_known_peers: peer_book.len(),
        peer_book_discovered_peers: peer_book.discovered_count(),
        relay_server_enabled: cfg.relay.is_active_now(),
        mediator_enabled: resolved_config.mediator_enabled,
        mediator_advertise_for_dcutr: resolved_config.mediator_advertise_for_dcutr,
        mediator_require_authenticated_peers: cfg.mediator.require_authenticated_peers,
        mediator_active_reservations: 0,
        mediator_active_circuits: 0,
        mediator_dcutr_attempts_observed: 0,
        mediator_denied_reservations: 0,
        mediator_denied_circuits: 0,
        mediator_abuse_rate_limit_events: 0,
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
        relay_discovery_enabled: relay_selection_plan.enabled,
        relay_discovery_min_reservations: relay_selection_plan.min_reservations,
        relay_discovery_max_reservations: relay_selection_plan.max_reservations,
        relay_discovery_selected_relays: relay_selection_plan.selected_strings(),
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
        relayed_listen_addresses: relay_reservation_plan
            .listen_addrs
            .iter()
            .map(ToString::to_string)
            .collect(),
        dcutr_enabled: resolved_config.dcutr_enabled,
        dcutr_attempt_after_relay_connection: resolved_config.dcutr_attempt_after_relay_connection,
        dcutr_keep_relay_fallback: resolved_config.dcutr_keep_relay_fallback,
        dcutr_retry_interval_secs: resolved_config.dcutr_retry_interval_secs,
        dcutr_max_attempts_per_peer: resolved_config.dcutr_max_attempts_per_peer,
        dcutr_attempts: 0,
        dcutr_successes: 0,
        dcutr_failures: 0,
        dcutr_relay_fallbacks: 0,
        dcutr_upgrade_eligible_connections: 0,
        dcutr_retry_suppressed: 0,
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
        app_subscriptions: Vec::new(),
        app_messages_sent: 0,
        app_messages_received: 0,
        app_messages_ignored: 0,
        app_messages_rejected: 0,
        api_commands_processed: 0,
        api_command_failures: 0,
        gossip_messages_rejected: 0,
        gossip_messages_ignored: 0,
        gossip_messages_accepted: 0,
        pulses: VecDeque::new(),
        uptime_secs: 0,
    }));

    {
        let mut guard = snapshot.lock().await;
        push_pulse(
            &mut guard.pulses,
            format!(
                "environment detected platform={} runtime={} storage={} reachability={} nat={} advisory_role={} mediator_enabled={}",
                environment_report.platform.as_str(),
                platform_runtime.runtime_name(),
                storage.storage_kind(),
                environment_report.reachability.as_str(),
                environment_report.nat_status.as_str(),
                resolved_config.role.as_str(),
                resolved_config.mediator_enabled
            ),
        );
    }

    if public_bootstrap_decision.used || public_relay_decision.used {
        let mut guard = snapshot.lock().await;
        push_pulse(
            &mut guard.pulses,
            format!(
                "public_fallback mode={} bootstrap_used={} relay_used={} bootstrap_reason={} relay_reason={}",
                cfg.discovery.public_bootstrap.mode.as_str(),
                public_bootstrap_decision.used,
                public_relay_decision.used,
                public_bootstrap_decision.reason,
                public_relay_decision.reason
            ),
        );
    }

    if relay_selection_plan.total_candidates() > 0
        || !relay_selection_plan.selected_addrs.is_empty()
        || !relay_selection_plan.errors.is_empty()
    {
        let mut guard = snapshot.lock().await;
        push_pulse(
            &mut guard.pulses,
            format!(
                "relay_discovery selected={} candidates={} configured={} cached={} rendezvous={} public={} ignored={}",
                relay_selection_plan.selected_addrs.len(),
                relay_selection_plan.total_candidates(),
                relay_selection_plan.configured_candidates,
                relay_selection_plan.cached_candidates,
                relay_selection_plan.rendezvous_candidates,
                relay_selection_plan.public_candidates,
                relay_selection_plan.ignored_candidates
            ),
        );
        for err in &relay_selection_plan.errors {
            push_pulse(&mut guard.pulses, format!("relay_discovery warning: {err}"));
        }
    }

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

    if dht_plan.enabled || !dht_plan.errors.is_empty() {
        let mut guard = snapshot.lock().await;
        for err in &dht_plan.errors {
            push_pulse(&mut guard.pulses, format!("dht provider startup error: {err}"));
        }
        if dht_plan.announce_attempts > 0 || dht_plan.provider_queries > 0 {
            push_pulse(
                &mut guard.pulses,
                format!(
                    "dht provider startup namespaces={} announce_attempts={} provider_queries={}",
                    dht_plan.namespace_count,
                    dht_plan.announce_attempts,
                    dht_plan.provider_queries
                ),
            );
        }
    }

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    let (command_tx, mut command_rx) = mpsc::channel(128);
    let (messages_tx, _) = broadcast::channel(256);
    let task_snapshot = Arc::clone(&snapshot);
    let rendezvous_peers_for_task = rendezvous_peers.clone();
    let storage_for_task = Arc::clone(&storage);
    let messages_for_task = messages_tx.clone();
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
        };
        let mut rendezvous_state = rendezvous_state;
        let mut dht_state = dht_state;
        let mut peer_book = peer_book;
        let mut pending_connections = PendingConnectionPlans::default();
        let mut connection_caps = ConnectionCapState::new(&cfg.connection_limits);
        let mut app_topic_hashes = Vec::new();
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
                maybe_command = command_rx.recv() => {
                    if let Some(command) = maybe_command {
                        commands::handle_node_command(
                            command,
                            &mut swarm,
                            local_peer,
                            cfg.network_id,
                            &mut app_topic_hashes,
                            &task_snapshot,
                            &mut peer_book,
                            &mut pending_connections,
                            &cfg.dcutr,
                        ).await;
                    } else {
                        break;
                    }
                }
                evt = swarm.select_next_some() => {
                    let mut event_ctx = events::SwarmEventContext {
                        snapshot: &task_snapshot,
                        rep: &mut rep,
                        relay_state: &mut relay_state,
                        rendezvous_state: &mut rendezvous_state,
                        dht_state: &mut dht_state,
                        peer_book: &mut peer_book,
                        pending_connections: &mut pending_connections,
                        connection_caps: &mut connection_caps,
                        relay_cfg: &cfg.relay,
                        dcutr_policy: &cfg.dcutr,
                        discovery_cfg: &cfg.discovery,
                        storage: storage_for_task.as_ref(),
                        rendezvous_peers: &rendezvous_peers_for_task,
                        message_security: &cfg.message_security,
                        replay_cache: &mut replay_cache,
                        heartbeat_topic_hash: &heartbeat_topic_hash,
                        app_topic_hashes: &app_topic_hashes,
                        app_messages: &messages_for_task,
                        local_peer,
                        network_id: cfg.network_id,
                    };
                    events::handle_swarm_event(evt, &mut swarm, &mut event_ctx).await;
                }
            }
        }
    });

    Ok(NodeHandle {
        peer_id: local_peer,
        snapshot,
        command_tx,
        messages_tx,
        shutdown_tx,
        task: Arc::new(Mutex::new(Some(task))),
    })
}


fn record_peer_book_addrs(peer_book: &mut PeerBook, addrs: &[Multiaddr], source: PeerSource) {
    for addr in addrs {
        if let Some(peer) = extract_p2p_peer_id(addr) {
            peer_book.record_addr(peer, addr.clone(), source);
        }
    }
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
    serde_json::to_value(snapshot).unwrap_or(Value::Null)
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
    line("p2p_peer_book_known_peers", snapshot.peer_book_known_peers, &mut out);
    line("p2p_peer_book_discovered_peers", snapshot.peer_book_discovered_peers, &mut out);
    line("p2p_discovery_namespace_count", snapshot.discovery_namespace_count, &mut out);
    line(
        "p2p_dht_provider_enabled",
        if snapshot.dht_provider_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_dht_provider_announce_attempts",
        snapshot.dht_provider_announce_attempts,
        &mut out,
    );
    line(
        "p2p_dht_provider_announce_failures",
        snapshot.dht_provider_announce_failures,
        &mut out,
    );
    line("p2p_dht_provider_queries", snapshot.dht_provider_queries, &mut out);
    line(
        "p2p_dht_provider_query_failures",
        snapshot.dht_provider_query_failures,
        &mut out,
    );
    line(
        "p2p_dht_provider_records_found",
        snapshot.dht_provider_records_found,
        &mut out,
    );
    line(
        "p2p_dht_provider_queries_finished",
        snapshot.dht_provider_queries_finished,
        &mut out,
    );
    line(
        "p2p_dht_provider_peers_discovered",
        snapshot.dht_provider_peers_discovered,
        &mut out,
    );
    line(
        "p2p_public_fallback_used",
        if snapshot.public_fallback_used { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_public_bootstrap_seed_count",
        snapshot.public_bootstrap_seed_count,
        &mut out,
    );
    line(
        "p2p_public_relay_candidate_count",
        snapshot.public_relay_candidate_count,
        &mut out,
    );
    line("p2p_api_commands_processed", snapshot.api_commands_processed, &mut out);
    line("p2p_api_command_failures", snapshot.api_command_failures, &mut out);
    line("p2p_app_subscriptions", snapshot.app_subscriptions.len(), &mut out);
    line("p2p_app_messages_sent", snapshot.app_messages_sent, &mut out);
    line("p2p_app_messages_received", snapshot.app_messages_received, &mut out);
    line("p2p_app_messages_ignored", snapshot.app_messages_ignored, &mut out);
    line("p2p_app_messages_rejected", snapshot.app_messages_rejected, &mut out);
    line(
        "p2p_platform_can_listen_tcp",
        if snapshot.platform_can_listen_tcp { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_platform_can_listen_quic",
        if snapshot.platform_can_listen_quic { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_relay_server_enabled",
        if snapshot.relay_server_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_mediator_enabled",
        if snapshot.mediator_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_mediator_active_reservations",
        snapshot.mediator_active_reservations,
        &mut out,
    );
    line(
        "p2p_mediator_active_circuits",
        snapshot.mediator_active_circuits,
        &mut out,
    );
    line(
        "p2p_mediator_dcutr_attempts_observed",
        snapshot.mediator_dcutr_attempts_observed,
        &mut out,
    );
    line(
        "p2p_mediator_denied_reservations",
        snapshot.mediator_denied_reservations,
        &mut out,
    );
    line(
        "p2p_mediator_denied_circuits",
        snapshot.mediator_denied_circuits,
        &mut out,
    );
    line(
        "p2p_mediator_abuse_rate_limit_events",
        snapshot.mediator_abuse_rate_limit_events,
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
    line(
        "p2p_relay_discovery_enabled",
        if snapshot.relay_discovery_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_relay_discovery_selected_relays",
        snapshot.relay_discovery_selected_relays.len(),
        &mut out,
    );
    line(
        "p2p_relay_discovery_candidate_count",
        snapshot.relay_discovery_candidate_count,
        &mut out,
    );
    line(
        "p2p_relay_discovery_public_candidates",
        snapshot.relay_discovery_public_candidates,
        &mut out,
    );
    line(
        "p2p_relay_discovery_failures",
        snapshot.relay_discovery_failures,
        &mut out,
    );
    line(
        "p2p_relay_discovery_replacements",
        snapshot.relay_discovery_replacements,
        &mut out,
    );
    line(
        "p2p_dcutr_enabled",
        if snapshot.dcutr_enabled { 1 } else { 0 },
        &mut out,
    );
    line("p2p_dcutr_attempts", snapshot.dcutr_attempts, &mut out);
    line("p2p_dcutr_successes", snapshot.dcutr_successes, &mut out);
    line("p2p_dcutr_failures", snapshot.dcutr_failures, &mut out);
    line(
        "p2p_dcutr_relay_fallbacks",
        snapshot.dcutr_relay_fallbacks,
        &mut out,
    );
    line(
        "p2p_dcutr_upgrade_eligible_connections",
        snapshot.dcutr_upgrade_eligible_connections,
        &mut out,
    );
    line(
        "p2p_dcutr_retry_suppressed",
        snapshot.dcutr_retry_suppressed,
        &mut out,
    );
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
