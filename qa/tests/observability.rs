use std::fs;
use std::time::Duration;

use libp2p::PeerId;
use p2p_net::connectivity::relay::{RelayServiceHealth, RelayState};
use p2p_net::{
    snapshot_to_json, snapshot_to_prometheus_metrics, start_node, DiscoveryConfig, NodeConfig,
    NodeSnapshot,
};

#[test]
fn snapshot_json_includes_accurate_relay_fields() {
    let snap = NodeSnapshot {
        peer_id: PeerId::random().to_string(),
        relay_server_enabled: true,
        mediator_enabled: true,
        mediator_advertise_for_dcutr: true,
        mediator_require_authenticated_peers: false,
        mediator_active_reservations: 2,
        mediator_active_circuits: 4,
        mediator_dcutr_attempts_observed: 6,
        mediator_denied_reservations: 1,
        mediator_denied_circuits: 2,
        mediator_abuse_rate_limit_events: 3,
        relay_service_health: RelayServiceHealth::Enabled,
        relay_reservations_accepted: 2,
        relay_client_reservations: 3,
        relay_discovery_enabled: true,
        relay_discovery_selected_relays: vec!["/ip4/127.0.0.1/tcp/4001/p2p/relay".to_string()],
        relay_discovery_candidate_count: 3,
        relay_discovery_public_candidates: 1,
        relay_discovery_failures: 1,
        relay_active_circuits: 4,
        relay_denied_requests: 5,
        relay_bytes_forwarded: 1234,
        dcutr_enabled: true,
        dcutr_attempt_after_relay_connection: true,
        dcutr_keep_relay_fallback: true,
        dcutr_retry_interval_secs: 60,
        dcutr_max_attempts_per_peer: 3,
        dcutr_attempts: 6,
        dcutr_successes: 2,
        dcutr_failures: 1,
        dcutr_relay_fallbacks: 4,
        dcutr_upgrade_eligible_connections: 5,
        dcutr_retry_suppressed: 1,
        platform_runtime: "desktop".to_string(),
        platform_storage: "desktop_fs".to_string(),
        platform_default_data_dir: Some("/tmp/p2p-net".to_string()),
        platform_can_listen_tcp: true,
        platform_can_listen_quic: true,
        active_transports: vec!["tcp".to_string(), "quic".to_string()],
        discovery_namespace_mode: "hashed".to_string(),
        discovery_namespaces: vec!["p2p-net/1/hydra-msg/hash".to_string()],
        discovery_namespace_count: 1,
        dht_provider_enabled: true,
        dht_provider_announce_enabled: true,
        dht_provider_discover_enabled: true,
        dht_provider_namespaces_announced: 1,
        dht_provider_announce_attempts: 2,
        dht_provider_announce_failures: 1,
        dht_provider_queries: 3,
        dht_provider_query_failures: 1,
        dht_provider_records_found: 4,
        dht_provider_queries_finished: 2,
        dht_provider_peers_discovered: 2,
        public_fallback_mode: "fallback_only".to_string(),
        public_fallback_used: true,
        public_fallback_reason: "no_operator_or_cached_startup_candidates".to_string(),
        public_bootstrap_seed_count: 2,
        public_relay_candidate_count: 1,
        peer_book_known_peers: 5,
        peer_book_discovered_peers: 3,
        app_subscriptions: vec!["chat/general".to_string()],
        app_messages_sent: 13,
        app_messages_received: 14,
        app_messages_ignored: 15,
        app_messages_rejected: 16,
        api_commands_processed: 17,
        api_command_failures: 1,
        ..NodeSnapshot::default()
    };

    let json = snapshot_to_json(&snap);
    assert_eq!(json["relay_server_enabled"].as_bool(), Some(true));
    assert_eq!(json["mediator_enabled"].as_bool(), Some(true));
    assert_eq!(json["mediator_active_reservations"], 2);
    assert_eq!(json["mediator_active_circuits"], 4);
    assert_eq!(json["mediator_dcutr_attempts_observed"], 6);
    assert_eq!(json["mediator_abuse_rate_limit_events"], 3);
    assert_eq!(json["relay_service_health"], "enabled");
    assert_eq!(json["relay_reservations_accepted"], 2);
    assert_eq!(json["relay_client_reservations"], 3);
    assert_eq!(json["relay_discovery_enabled"].as_bool(), Some(true));
    assert_eq!(json["relay_discovery_candidate_count"], 3);
    assert_eq!(json["relay_discovery_public_candidates"], 1);
    assert_eq!(json["relay_discovery_failures"], 1);
    assert_eq!(json["relay_active_circuits"], 4);
    assert_eq!(json["relay_denied_requests"], 5);
    assert_eq!(json["relay_bytes_forwarded"], 1234);
    assert_eq!(json["dcutr_enabled"].as_bool(), Some(true));
    assert_eq!(json["dcutr_keep_relay_fallback"].as_bool(), Some(true));
    assert_eq!(json["dcutr_retry_interval_secs"], 60);
    assert_eq!(json["dcutr_max_attempts_per_peer"], 3);
    assert_eq!(json["dcutr_attempts"], 6);
    assert_eq!(json["dcutr_successes"], 2);
    assert_eq!(json["dcutr_failures"], 1);
    assert_eq!(json["dcutr_relay_fallbacks"], 4);
    assert_eq!(json["dcutr_upgrade_eligible_connections"], 5);
    assert_eq!(json["dcutr_retry_suppressed"], 1);
    assert_eq!(json["platform_runtime"], "desktop");
    assert_eq!(json["platform_storage"], "desktop_fs");
    assert_eq!(json["platform_default_data_dir"], "/tmp/p2p-net");
    assert_eq!(json["platform_can_listen_tcp"].as_bool(), Some(true));
    assert_eq!(json["platform_can_listen_quic"].as_bool(), Some(true));
    assert_eq!(json["discovery_namespace_mode"], "hashed");
    assert_eq!(json["discovery_namespace_count"], 1);
    assert_eq!(json["discovery_namespaces"][0], "p2p-net/1/hydra-msg/hash");
    assert_eq!(json["dht_provider_enabled"].as_bool(), Some(true));
    assert_eq!(json["dht_provider_announce_enabled"].as_bool(), Some(true));
    assert_eq!(json["dht_provider_discover_enabled"].as_bool(), Some(true));
    assert_eq!(json["dht_provider_namespaces_announced"], 1);
    assert_eq!(json["dht_provider_announce_attempts"], 2);
    assert_eq!(json["dht_provider_announce_failures"], 1);
    assert_eq!(json["dht_provider_queries"], 3);
    assert_eq!(json["dht_provider_query_failures"], 1);
    assert_eq!(json["dht_provider_records_found"], 4);
    assert_eq!(json["dht_provider_queries_finished"], 2);
    assert_eq!(json["dht_provider_peers_discovered"], 2);
    assert_eq!(json["public_fallback_mode"], "fallback_only");
    assert_eq!(json["public_fallback_used"].as_bool(), Some(true));
    assert_eq!(json["public_bootstrap_seed_count"], 2);
    assert_eq!(json["public_relay_candidate_count"], 1);
    assert_eq!(json["peer_book_known_peers"], 5);
    assert_eq!(json["peer_book_discovered_peers"], 3);
    assert_eq!(json["app_subscriptions"][0], "chat/general");
    assert_eq!(json["app_messages_sent"], 13);
    assert_eq!(json["app_messages_received"], 14);
    assert_eq!(json["app_messages_ignored"], 15);
    assert_eq!(json["app_messages_rejected"], 16);
    assert_eq!(json["api_commands_processed"], 17);
    assert_eq!(json["api_command_failures"], 1);
    assert!(json.get("relay_reservations").is_none());
    assert!(json.get("relay_circuits").is_none());
    assert!(json.get("dcutr_events").is_none());
}

#[test]
fn prometheus_metrics_exports_operator_counters() {
    let snap = NodeSnapshot {
        connected_peers: 7,
        peer_book_known_peers: 5,
        peer_book_discovered_peers: 3,
        discovery_namespace_count: 2,
        dht_provider_enabled: true,
        dht_provider_announce_attempts: 2,
        dht_provider_announce_failures: 1,
        dht_provider_queries: 3,
        dht_provider_query_failures: 1,
        dht_provider_records_found: 4,
        dht_provider_queries_finished: 2,
        dht_provider_peers_discovered: 2,
        public_fallback_used: true,
        public_bootstrap_seed_count: 2,
        public_relay_candidate_count: 1,
        app_subscriptions: vec!["chat/general".to_string()],
        app_messages_sent: 13,
        app_messages_received: 14,
        app_messages_ignored: 15,
        app_messages_rejected: 16,
        api_commands_processed: 17,
        api_command_failures: 1,
        relay_server_enabled: true,
        mediator_enabled: true,
        mediator_active_reservations: 2,
        mediator_active_circuits: 4,
        mediator_dcutr_attempts_observed: 6,
        mediator_denied_reservations: 1,
        mediator_denied_circuits: 2,
        mediator_abuse_rate_limit_events: 3,
        relay_reservations_accepted: 2,
        relay_client_reservations: 3,
        relay_discovery_enabled: true,
        relay_discovery_selected_relays: vec!["/ip4/127.0.0.1/tcp/4001/p2p/relay".to_string()],
        relay_discovery_candidate_count: 3,
        relay_discovery_public_candidates: 1,
        relay_discovery_failures: 1,
        relay_active_circuits: 4,
        relay_denied_requests: 5,
        relay_bytes_forwarded: 1234,
        dcutr_enabled: true,
        dcutr_attempts: 6,
        dcutr_successes: 2,
        dcutr_failures: 1,
        dcutr_relay_fallbacks: 4,
        dcutr_upgrade_eligible_connections: 5,
        dcutr_retry_suppressed: 1,
        gossip_messages_accepted: 10,
        gossip_messages_ignored: 11,
        gossip_messages_rejected: 12,
        platform_can_listen_tcp: true,
        platform_can_listen_quic: true,
        ..NodeSnapshot::default()
    };

    let metrics = snapshot_to_prometheus_metrics(&snap);
    assert!(metrics.contains("p2p_connected_peers 7\n"));
    assert!(metrics.contains("p2p_peer_book_known_peers 5\n"));
    assert!(metrics.contains("p2p_peer_book_discovered_peers 3\n"));
    assert!(metrics.contains("p2p_discovery_namespace_count 2\n"));
    assert!(metrics.contains("p2p_dht_provider_enabled 1\n"));
    assert!(metrics.contains("p2p_dht_provider_announce_attempts 2\n"));
    assert!(metrics.contains("p2p_dht_provider_announce_failures 1\n"));
    assert!(metrics.contains("p2p_dht_provider_queries 3\n"));
    assert!(metrics.contains("p2p_dht_provider_query_failures 1\n"));
    assert!(metrics.contains("p2p_dht_provider_records_found 4\n"));
    assert!(metrics.contains("p2p_dht_provider_queries_finished 2\n"));
    assert!(metrics.contains("p2p_dht_provider_peers_discovered 2\n"));
    assert!(metrics.contains("p2p_public_fallback_used 1\n"));
    assert!(metrics.contains("p2p_public_bootstrap_seed_count 2\n"));
    assert!(metrics.contains("p2p_public_relay_candidate_count 1\n"));
    assert!(metrics.contains("p2p_api_commands_processed 17\n"));
    assert!(metrics.contains("p2p_api_command_failures 1\n"));
    assert!(metrics.contains("p2p_app_subscriptions 1\n"));
    assert!(metrics.contains("p2p_app_messages_sent 13\n"));
    assert!(metrics.contains("p2p_app_messages_received 14\n"));
    assert!(metrics.contains("p2p_app_messages_ignored 15\n"));
    assert!(metrics.contains("p2p_app_messages_rejected 16\n"));
    assert!(metrics.contains("p2p_platform_can_listen_tcp 1\n"));
    assert!(metrics.contains("p2p_platform_can_listen_quic 1\n"));
    assert!(metrics.contains("p2p_relay_server_enabled 1\n"));
    assert!(metrics.contains("p2p_mediator_enabled 1\n"));
    assert!(metrics.contains("p2p_mediator_active_reservations 2\n"));
    assert!(metrics.contains("p2p_mediator_active_circuits 4\n"));
    assert!(metrics.contains("p2p_mediator_dcutr_attempts_observed 6\n"));
    assert!(metrics.contains("p2p_mediator_denied_reservations 1\n"));
    assert!(metrics.contains("p2p_mediator_denied_circuits 2\n"));
    assert!(metrics.contains("p2p_mediator_abuse_rate_limit_events 3\n"));
    assert!(metrics.contains("p2p_relay_reservations_accepted 2\n"));
    assert!(metrics.contains("p2p_relay_client_reservations 3\n"));
    assert!(metrics.contains("p2p_relay_discovery_enabled 1\n"));
    assert!(metrics.contains("p2p_relay_discovery_selected_relays 1\n"));
    assert!(metrics.contains("p2p_relay_discovery_candidate_count 3\n"));
    assert!(metrics.contains("p2p_relay_discovery_public_candidates 1\n"));
    assert!(metrics.contains("p2p_relay_discovery_failures 1\n"));
    assert!(metrics.contains("p2p_relay_active_circuits 4\n"));
    assert!(metrics.contains("p2p_relay_denied_requests 5\n"));
    assert!(metrics.contains("p2p_relay_bytes_forwarded 1234\n"));
    assert!(metrics.contains("p2p_dcutr_enabled 1\n"));
    assert!(metrics.contains("p2p_dcutr_attempts 6\n"));
    assert!(metrics.contains("p2p_dcutr_successes 2\n"));
    assert!(metrics.contains("p2p_dcutr_failures 1\n"));
    assert!(metrics.contains("p2p_dcutr_relay_fallbacks 4\n"));
    assert!(metrics.contains("p2p_dcutr_upgrade_eligible_connections 5\n"));
    assert!(metrics.contains("p2p_dcutr_retry_suppressed 1\n"));
}

#[test]
fn relay_state_updates_snapshot_counters_without_mixing_meanings() {
    let peer = PeerId::random();
    let mut state = RelayState {
        server_enabled: true,
        health: RelayServiceHealth::AtCapacity,
        accepted_reservations: 2,
        active_circuits: 4,
        denied_reservations: 3,
        denied_circuits: 5,
        rate_limited_events: 6,
        at_capacity_events: 7,
        server_errors: 8,
        relay_client_reservation_attempts: 9,
        relay_client_reservation_failures: 1,
        relay_discovery_candidate_count: 3,
        relay_discovery_public_candidates: 1,
        relay_discovery_failures: 1,
        relay_bytes_forwarded: 1234,
        dcutr_enabled: true,
        dcutr_attempts: 10,
        dcutr_successes: 2,
        dcutr_failures: 1,
        dcutr_relay_fallbacks: 3,
        dcutr_upgrade_eligible_connections: 4,
        dcutr_retry_suppressed: 1,
        ..RelayState::default()
    };
    state.relay_client_reservations.insert(peer);

    let mut snap = NodeSnapshot {
        mediator_enabled: true,
        ..NodeSnapshot::default()
    };
    snap.apply_relay_state(&state);

    assert!(snap.relay_server_enabled);
    assert_eq!(snap.relay_service_health, RelayServiceHealth::AtCapacity);
    assert_eq!(snap.relay_reservations_accepted, 2);
    assert_eq!(snap.relay_client_reservations, 1);
    assert_eq!(snap.relay_discovery_candidate_count, 3);
    assert_eq!(snap.relay_discovery_public_candidates, 1);
    assert_eq!(snap.relay_discovery_failures, 1);
    assert_eq!(snap.relay_active_circuits, 4);
    assert_eq!(snap.relay_denied_reservations, 3);
    assert_eq!(snap.relay_denied_circuits, 5);
    assert_eq!(snap.relay_denied_requests, 8);
    assert_eq!(snap.relay_bytes_forwarded, 1234);
    assert_eq!(snap.mediator_active_reservations, 2);
    assert_eq!(snap.mediator_active_circuits, 4);
    assert_eq!(snap.mediator_dcutr_attempts_observed, 10);
    assert_eq!(snap.mediator_denied_reservations, 3);
    assert_eq!(snap.mediator_denied_circuits, 5);
    assert_eq!(snap.mediator_abuse_rate_limit_events, 13);
    assert!(snap.dcutr_enabled);
    assert_eq!(snap.dcutr_attempts, 10);
    assert_eq!(snap.dcutr_successes, 2);
    assert_eq!(snap.dcutr_failures, 1);
    assert_eq!(snap.dcutr_relay_fallbacks, 3);
    assert_eq!(snap.dcutr_upgrade_eligible_connections, 4);
    assert_eq!(snap.dcutr_retry_suppressed, 1);
}

#[tokio::test]
async fn dashboard_does_not_claim_inactive_or_unimplemented_transports() {
    let cfg = NodeConfig {
        identity_key_path: temp_path("observability-transports-key")
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        discovery: DiscoveryConfig {
            peer_cache_path: temp_path("observability-transports-cache")
                .to_string_lossy()
                .to_string(),
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("start node");
    let active = handle.snapshot.lock().await.active_transports.clone();
    handle.shutdown().await;
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(cache_path);

    assert!(!active.iter().any(|name| name == "webrtc-direct"));
    assert!(!active.iter().any(|name| name == "webtransport"));
    assert!(!active.iter().any(|name| name == "relay-server"));
    assert!(!active.iter().any(|name| name == "rendezvous-client"));
    assert!(!active.iter().any(|name| name == "rendezvous-server"));
}

#[tokio::test]
async fn shutdown_exits_swarm_task_cleanly() {
    let cfg = NodeConfig {
        identity_key_path: temp_path("observability-shutdown-key")
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        discovery: DiscoveryConfig {
            peer_cache_path: temp_path("observability-shutdown-cache")
                .to_string_lossy()
                .to_string(),
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("start node");

    tokio::time::timeout(Duration::from_secs(2), handle.shutdown())
        .await
        .expect("shutdown should not hang");
    tokio::time::timeout(Duration::from_secs(2), handle.shutdown())
        .await
        .expect("second shutdown should be idempotent");

    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(cache_path);
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("p2p-net-{prefix}-{}", PeerId::random()))
}
