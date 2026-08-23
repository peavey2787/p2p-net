use std::fs;
use std::time::Duration;

use libp2p::PeerId;
use p2p_net::{
    start_node, DhtDiscoveryConfig, DiscoveryConfig, NodeConfig, NodeHandle, NodeProfile,
    NodeSnapshot, PublicBootstrapConfig, PublicIpProbeConfig, RelayServiceConfig, RendezvousConfig,
};

const LISTEN_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RELAY_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn rapid_connect_disconnect_loop_does_not_hang_shutdown() {
    for i in 0..10u32 {
        let cfg = test_config(&format!("rapid-{i}"));
        let key_path = cfg.identity_key_path.clone();
        let cache_path = cfg.discovery.peer_cache_path.clone();
        let handle = start_node(cfg).await.expect("start node");
        tokio::time::timeout(Duration::from_secs(2), handle.shutdown())
            .await
            .expect("shutdown should not hang");
        cleanup_files(&key_path, &cache_path);
    }
}

#[tokio::test]
async fn public_relay_node_starts_with_abuse_limits_enabled() {
    let cfg = relay_server_config("public-relay");
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("start relay node");
    let snap = handle.snapshot.lock().await.clone();
    handle.shutdown().await;
    cleanup_files(&key_path, &cache_path);

    assert!(snap.relay_server_enabled);
    assert_eq!(snap.relay_acl_scope, "connection_level");
    assert!(snap.active_transports.iter().any(|v| v == "relay-server"));
    assert!(snap
        .active_transports
        .iter()
        .any(|v| v == "connection-limits"));
}

#[tokio::test]
#[ignore = "deferred hostile relay-load test; run-full-validation always executes it in the final long-test phase"]
async fn relay_reservation_spam_does_not_panic() {
    let relay_cfg = relay_server_config("relay-spam-server");
    let relay_key_path = relay_cfg.identity_key_path.clone();
    let relay_cache_path = relay_cfg.discovery.peer_cache_path.clone();
    let relay = start_node(relay_cfg)
        .await
        .expect("start local relay server");
    let (relay_addr, _) = wait_for_tcp_dial_addr(&relay).await;

    let mut clients = Vec::new();

    // Establish one real reservation before applying the hostile burst. This keeps
    // the test honest: connection-level abuse limits may reject later clients
    // before the relay protocol sees them, but the relay path itself must work.
    let control_cfg = NodeConfig {
        relay_peers: vec![relay_addr.clone()],
        reserve_configured_relays: true,
        ..test_config("relay-spam-client-0")
    };
    let control_key_path = control_cfg.identity_key_path.clone();
    let control_cache_path = control_cfg.discovery.peer_cache_path.clone();
    let control = start_node(control_cfg)
        .await
        .expect("start relay control client");
    clients.push((control, control_key_path, control_cache_path));

    let control_accepted = wait_for_relay_acceptance(&relay).await;
    let baseline = relay.snapshot.lock().await.clone();

    for i in 1..16u32 {
        let client_cfg = NodeConfig {
            relay_peers: vec![relay_addr.clone()],
            reserve_configured_relays: true,
            ..test_config(&format!("relay-spam-client-{i}"))
        };
        let key_path = client_cfg.identity_key_path.clone();
        let cache_path = client_cfg.discovery.peer_cache_path.clone();
        let handle = start_node(client_cfg).await.expect("start relay client");
        clients.push((handle, key_path, cache_path));
    }

    let saw_relay_activity = wait_for_hostile_relay_defense(&relay, &baseline).await;
    let relay_responsive = tokio::time::timeout(Duration::from_secs(2), relay.get_peers())
        .await
        .is_ok_and(|result| result.is_ok());
    let relay_snapshot = relay.snapshot.lock().await.clone();

    for (handle, key_path, cache_path) in clients {
        handle.shutdown().await;
        cleanup_files(&key_path, &cache_path);
    }
    relay.shutdown().await;
    cleanup_files(&relay_key_path, &relay_cache_path);

    assert!(
        control_accepted,
        "local relay never accepted the control reservation before the hostile burst; \
accepted_total={} denied={} server_errors={} pulses={:?}",
        baseline.relay_reservations_accepted_total,
        baseline.relay_denied_reservations,
        baseline.relay_server_errors,
        baseline.pulses,
    );
    assert!(
        saw_relay_activity,
        "local relay observed no defensive-state change from the hostile client burst; \
baseline accepted_total={} denied={} rate_limited={} at_capacity={} server_errors={} cap_disconnects={}; \
after accepted_total={} denied={} rate_limited={} at_capacity={} server_errors={} cap_disconnects={} pulses={:?}",
        baseline.relay_reservations_accepted_total,
        baseline.relay_denied_reservations,
        baseline.relay_rate_limited_events,
        baseline.relay_at_capacity_events,
        baseline.relay_server_errors,
        baseline.connection_cap_disconnects,
        relay_snapshot.relay_reservations_accepted_total,
        relay_snapshot.relay_denied_reservations,
        relay_snapshot.relay_rate_limited_events,
        relay_snapshot.relay_at_capacity_events,
        relay_snapshot.relay_server_errors,
        relay_snapshot.connection_cap_disconnects,
        relay_snapshot.pulses,
    );
    assert!(
        relay_responsive,
        "local relay stopped responding to API commands during the hostile reservation burst"
    );
}

#[tokio::test]
#[ignore = "deferred hostile connection-churn test; run-full-validation always executes it after normal checks"]
async fn circuit_open_close_spam_does_not_hang() {
    let target_cfg = test_config("open-close-target");
    let target_key_path = target_cfg.identity_key_path.clone();
    let target_cache_path = target_cfg.discovery.peer_cache_path.clone();
    let target = start_node(target_cfg)
        .await
        .expect("start local churn target");
    let (target_addr, target_peer_id) = wait_for_tcp_dial_addr(&target).await;

    let mut failed_iteration = None;
    for i in 0..16u32 {
        let cfg = NodeConfig {
            bootstrap_peers: vec![target_addr.clone()],
            ..test_config(&format!("open-close-spam-{i}"))
        };
        let key_path = cfg.identity_key_path.clone();
        let cache_path = cfg.discovery.peer_cache_path.clone();
        let handle = start_node(cfg).await.expect("start churn dialer");
        let connected = wait_for_peer_connection(&handle, &target_peer_id).await;
        handle.shutdown().await;
        cleanup_files(&key_path, &cache_path);
        if !connected {
            failed_iteration = Some(i);
            break;
        }
    }

    target.shutdown().await;
    cleanup_files(&target_key_path, &target_cache_path);

    assert!(
        failed_iteration.is_none(),
        "local target was not reached during churn iteration {:?}",
        failed_iteration
    );
}

#[tokio::test]
#[ignore = "deferred one-minute soak test; run-full-validation executes it last"]
async fn long_running_soak_node_stays_responsive() {
    let cfg = test_config("soak");
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("start node");

    for sample in 1..=12u32 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        tokio::time::timeout(Duration::from_secs(2), handle.get_peers())
            .await
            .unwrap_or_else(|_| panic!("soak responsiveness probe {sample} timed out"))
            .unwrap_or_else(|err| panic!("soak responsiveness probe {sample} failed: {err}"));
        let snapshot_guard = tokio::time::timeout(Duration::from_secs(2), handle.snapshot.lock())
            .await
            .unwrap_or_else(|_| panic!("soak snapshot probe {sample} timed out"));
        drop(snapshot_guard);
    }

    let snap = handle.snapshot.lock().await.clone();
    handle.shutdown().await;
    cleanup_files(&key_path, &cache_path);

    assert!(snap.uptime_secs <= 120);
}

fn relay_server_config(label: &str) -> NodeConfig {
    NodeConfig {
        relay: RelayServiceConfig {
            enabled: true,
            max_reservations: 8,
            max_reservations_per_peer: 1,
            max_circuits: 16,
            max_circuits_per_peer: 2,
            max_circuit_bytes: 1024 * 1024,
            max_circuit_duration_secs: 30,
            reservation_rate_per_peer_per_min: 2,
            reservation_rate_per_ip_per_min: 8,
            circuit_rate_per_peer_per_min: 4,
            circuit_rate_per_ip_per_min: 16,
            ..RelayServiceConfig::default()
        },
        ..test_config(label)
    }
}

fn test_config(label: &str) -> NodeConfig {
    NodeConfig {
        profile: NodeProfile::Full,
        identity_key_path: temp_path(&format!("{label}-key"))
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        bootstrap_peers: Vec::new(),
        relay_peers: Vec::new(),
        discovery: DiscoveryConfig {
            peer_cache_path: temp_path(&format!("{label}-cache"))
                .to_string_lossy()
                .to_string(),
            public_bootstrap: PublicBootstrapConfig::private_infrastructure_only(),
            rendezvous: RendezvousConfig {
                client_enabled: false,
                server_enabled: false,
                ..RendezvousConfig::default()
            },
            dht: DhtDiscoveryConfig {
                enabled: false,
                announce: false,
                discover: false,
                ..DhtDiscoveryConfig::default()
            },
            ..DiscoveryConfig::default()
        },
        public_ip_probe: PublicIpProbeConfig {
            enabled: false,
            ..PublicIpProbeConfig::default()
        },
        heartbeat_interval_secs: 1,
        ..NodeConfig::default()
    }
}

async fn wait_for_tcp_dial_addr(handle: &NodeHandle) -> (String, PeerId) {
    tokio::time::timeout(LISTEN_TIMEOUT, async {
        loop {
            let snapshot = handle.snapshot.lock().await;
            if let Some(addr) = snapshot
                .local_listen_addresses
                .iter()
                .chain(snapshot.public_direct_listen_addresses.iter())
                .find(|addr| addr.contains("/tcp/") && !addr.contains("/ws"))
            {
                let peer_id = handle.peer_id;
                return (format!("{addr}/p2p/{peer_id}"), peer_id);
            }
            drop(snapshot);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("timed out waiting for local TCP listen address")
}

async fn wait_for_peer_connection(handle: &NodeHandle, peer_id: &PeerId) -> bool {
    let peer_id = peer_id.to_string();
    tokio::time::timeout(CONNECT_TIMEOUT, async {
        loop {
            if handle.get_peers().await.is_ok_and(|peers| {
                peers
                    .iter()
                    .any(|peer| peer.connected && peer.peer_id == peer_id)
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .is_ok()
}

async fn wait_for_relay_acceptance(handle: &NodeHandle) -> bool {
    tokio::time::timeout(RELAY_ACTIVITY_TIMEOUT, async {
        loop {
            if handle
                .snapshot
                .lock()
                .await
                .relay_reservations_accepted_total
                > 0
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .is_ok()
}

async fn wait_for_hostile_relay_defense(handle: &NodeHandle, baseline: &NodeSnapshot) -> bool {
    tokio::time::timeout(RELAY_ACTIVITY_TIMEOUT, async {
        loop {
            let snapshot = handle.snapshot.lock().await;
            let changed = snapshot.relay_reservations_accepted_total
                > baseline.relay_reservations_accepted_total
                || snapshot.relay_denied_reservations > baseline.relay_denied_reservations
                || snapshot.relay_rate_limited_events > baseline.relay_rate_limited_events
                || snapshot.relay_at_capacity_events > baseline.relay_at_capacity_events
                || snapshot.relay_server_errors > baseline.relay_server_errors
                || snapshot.connection_cap_disconnects > baseline.connection_cap_disconnects;
            if changed {
                return;
            }
            drop(snapshot);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .is_ok()
}

fn cleanup_files(key_path: &str, cache_path: &str) {
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(cache_path);
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("p2p-net-hostile-{prefix}-{}", PeerId::random()))
}
