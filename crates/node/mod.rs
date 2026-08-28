//! Standalone node orchestration: libp2p swarm ownership, command routing, events, and snapshots.

mod capabilities;
mod commands;
mod config;
mod config_validation;
mod dial;
mod environment;
mod events;
mod handle;
mod metrics;
mod profile;
mod public_ip;
mod runtime;
mod runtime_maintenance;
mod runtime_tasks;
mod snapshot;
mod startup;

use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use libp2p::gossipsub::IdentTopic;
use libp2p::PeerId;
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::common::error::NetError;
use crate::connectivity::dht::{start_dht_namespace_discovery, DhtProviderState};
use crate::connectivity::identity;
use crate::connectivity::rendezvous::RendezvousState;
use crate::platform::{DesktopPlatformRuntime, NodeStorage, PlatformRuntime};
use crate::protocol::pulse::heartbeat_topic;
use crate::stack::{
    allow_dcutr_peer, build_swarm, refresh_rendezvous, reserve_selected_relays, seed_bootstrap,
};

pub use capabilities::{apply_resolved_capabilities, resolve_node_config};
pub use config::{ListenerConfig, NodeConfig};
pub use environment::{
    EnvironmentConfig, EnvironmentReport, NatKind, NetworkReachability, PlatformKind,
};
pub use handle::NodeHandle;
pub use metrics::snapshot_to_prometheus_metrics;
pub use profile::{BehaviourSet, NodeProfile, NodeRole, ResolvedNodeConfig};
pub use public_ip::PublicIpProbeConfig;
pub use snapshot::{snapshot_to_json, NodeSnapshot};

use snapshot::network_label;

pub async fn start_node(cfg: NodeConfig) -> Result<NodeHandle, NetError> {
    let desktop = Arc::new(DesktopPlatformRuntime::default());
    let runtime: Arc<dyn PlatformRuntime> = desktop.clone();
    let storage: Arc<dyn NodeStorage> = desktop;
    start_node_with_platform(cfg, runtime, storage).await
}

/// Start with platform runtime/storage while keeping Android/iOS/Desktop shells
/// on the shared node logic and platform-specific lifecycle/data-directory adapters.
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
    let discovery_signing_key = local_key.clone();
    let (mut swarm, transport_plan) = build_swarm(local_key, &cfg, &resolved_config).await?;

    let heartbeat_topic = IdentTopic::new(heartbeat_topic(cfg.network_id));
    let _ = swarm.behaviour_mut().gossipsub.subscribe(&heartbeat_topic);

    let startup::StartupDiscoverySetup {
        startup_plan,
        rendezvous_peers,
        peer_book,
        relay_selection_plan,
        public_bootstrap_decision,
        public_rendezvous_decision,
        public_relay_decision,
        public_bootstrap_candidate_count,
        public_rendezvous_candidate_count,
        public_relay_candidate_count,
    } = startup::prepare_startup_discovery(&cfg, &resolved_config, storage.as_ref()).await?;

    for record in peer_book.records() {
        if !record.namespaces.is_empty()
            || record.sources.iter().any(|source| {
                matches!(
                    source,
                    crate::api::PeerSource::Manual
                        | crate::api::PeerSource::PeerCache
                        | crate::api::PeerSource::DhtProvider
                        | crate::api::PeerSource::LanDiscovery
                        | crate::api::PeerSource::Rendezvous
                        | crate::api::PeerSource::PublicRendezvous
                )
            })
        {
            allow_dcutr_peer(&mut swarm, record.peer_id);
        }
    }

    seed_bootstrap(&mut swarm, &startup_plan.dial_addrs);
    let selected_relay_peers = relay_selection_plan.selected_addrs.clone();
    let relay_reservation_plan =
        if resolved_config.should_reserve_selected_relays && !selected_relay_peers.is_empty() {
            reserve_selected_relays(&mut swarm, &selected_relay_peers)
        } else {
            if resolved_config.should_seed_selected_relays && !selected_relay_peers.is_empty() {
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
    } else if cfg.discovery.rendezvous.namespace
        == crate::connectivity::rendezvous::RendezvousConfig::default().namespace
    {
        "network_default".to_string()
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
    if public_rendezvous_decision.used {
        active_transports.push("public-rendezvous-fallback".to_string());
    }
    if public_relay_decision.used {
        active_transports.push("public-relay-fallback".to_string());
    }
    if cfg.public_ip_probe.enabled {
        active_transports.push("public-ip-probe".to_string());
    }
    if cfg.discovery.lan.enabled {
        active_transports.push("lan-discovery".to_string());
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

    let snapshot_revision = Arc::new(AtomicU64::new(1));
    let snapshot = Arc::new(Mutex::new(NodeSnapshot {
        network_id: cfg.network_id,
        network_label: network_label(cfg.network_id),
        peer_id: local_peer.to_string(),
        nat_status: "unknown".to_string(),
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
        public_fallback_used: public_bootstrap_decision.used
            || public_rendezvous_decision.used
            || public_relay_decision.used,
        public_fallback_reason: if public_bootstrap_decision.used {
            public_bootstrap_decision.reason.clone()
        } else if public_rendezvous_decision.used {
            public_rendezvous_decision.reason.clone()
        } else if public_relay_decision.used {
            public_relay_decision.reason.clone()
        } else {
            "not_used".to_string()
        },
        public_bootstrap_used: public_bootstrap_decision.used,
        public_bootstrap_reason: public_bootstrap_decision.reason.clone(),
        public_rendezvous_used: public_rendezvous_decision.used,
        public_rendezvous_reason: public_rendezvous_decision.reason.clone(),
        public_relay_used: public_relay_decision.used,
        public_relay_reason: public_relay_decision.reason.clone(),
        public_bootstrap_seed_count: public_bootstrap_candidate_count
            .max(startup_plan.public_bootstrap_seed_count),
        public_rendezvous_candidate_count,
        public_relay_candidate_count: public_relay_candidate_count
            .max(relay_selection_plan.public_candidates),
        public_ip_probe_enabled: cfg.public_ip_probe.enabled,
        public_ip_probe_status: if cfg.public_ip_probe.enabled {
            "pending".to_string()
        } else {
            "disabled".to_string()
        },
        peer_book_known_peers: peer_book.len(),
        peer_book_discovered_peers: peer_book.discovered_count(),
        auto_connect_enabled: cfg.discovery.public_bootstrap.auto_connect_discovered_peers,
        relay_server_enabled: cfg.relay.is_active_now(),
        mediator_enabled: resolved_config.mediator_enabled,
        mediator_advertise_for_dcutr: resolved_config.mediator_advertise_for_dcutr,
        mediator_require_authenticated_peers: cfg.mediator.require_authenticated_peers,
        relay_service_health: cfg.relay.health_now(),
        relay_acl_scope: "connection_level".to_string(),
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
        // Reservation listen addresses are requests, not confirmed relay
        // reachability. `NewListenAddr` populates this after acceptance.
        relayed_listen_addresses: Vec::new(),
        dcutr_enabled: resolved_config.dcutr_enabled,
        dcutr_attempt_after_relay_connection: resolved_config.dcutr_attempt_after_relay_connection,
        dcutr_keep_relay_fallback: resolved_config.dcutr_keep_relay_fallback,
        dcutr_retry_interval_secs: resolved_config.dcutr_retry_interval_secs,
        dcutr_max_attempts_per_peer: resolved_config.dcutr_max_attempts_per_peer,
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
        ..NodeSnapshot::default()
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

    if public_bootstrap_decision.used
        || public_rendezvous_decision.used
        || public_relay_decision.used
    {
        let mut guard = snapshot.lock().await;
        push_pulse(
            &mut guard.pulses,
            format!(
                "public_fallback mode={} bootstrap_used={} rendezvous_used={} relay_used={} auto_connect_discovered_peers={} bootstrap_reason={} rendezvous_reason={} relay_reason={}",
                cfg.discovery.public_bootstrap.mode.as_str(),
                public_bootstrap_decision.used,
                public_rendezvous_decision.used,
                public_relay_decision.used,
                cfg.discovery.public_bootstrap.auto_connect_discovered_peers,
                public_bootstrap_decision.reason,
                public_rendezvous_decision.reason,
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
            push_pulse(
                &mut guard.pulses,
                format!("dht provider startup error: {err}"),
            );
        }
        if dht_plan.announce_attempts > 0 || dht_plan.provider_queries > 0 {
            push_pulse(
                &mut guard.pulses,
                format!(
                    "dht provider startup namespaces={} announce_attempts={} provider_queries={}",
                    dht_plan.namespace_count, dht_plan.announce_attempts, dht_plan.provider_queries
                ),
            );
        }
    }

    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let (command_tx, command_rx) = mpsc::channel(128);
    let (messages_tx, _) = broadcast::channel(256);
    let task = runtime::spawn_node_runtime(runtime::NodeRuntimeContext {
        cfg: cfg.clone(),
        resolved_config,
        swarm,
        local_peer,
        discovery_signing_key,
        heartbeat_topic,
        snapshot: Arc::clone(&snapshot),
        snapshot_revision: Arc::clone(&snapshot_revision),
        storage,
        rendezvous_peers,
        relay_reservation_plan,
        relay_selection_plan,
        rendezvous_state,
        dht_state,
        peer_book,
        shutdown_rx,
        command_rx,
        messages_tx: messages_tx.clone(),
    });

    Ok(NodeHandle {
        peer_id: local_peer,
        snapshot,
        snapshot_revision,
        command_tx,
        messages_tx,
        shutdown_tx,
        task: Arc::new(Mutex::new(Some(task))),
        dnsaddr: cfg.dnsaddr,
    })
}

pub(crate) fn push_pulse(buf: &mut VecDeque<String>, line: String) {
    tracing::info!(target: "p2p_net::event", event = %line);
    buf.push_front(line);
    while buf.len() > 24 {
        let _ = buf.pop_back();
    }
}
