use std::fs;
use std::time::Duration;

use libp2p::PeerId;
use p2p_net::{start_node, DiscoveryConfig, NodeConfig, RelayServiceConfig};

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
        let _ = fs::remove_file(key_path);
        let _ = fs::remove_file(cache_path);
    }
}

#[tokio::test]
async fn public_relay_node_starts_with_abuse_limits_enabled() {
    let cfg = NodeConfig {
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
        ..test_config("public-relay")
    };
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("start relay node");
    let snap = handle.snapshot.lock().await.clone();
    handle.shutdown().await;
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(cache_path);

    assert!(snap.relay_server_enabled);
    assert_eq!(snap.relay_acl_scope, "connection_level");
    assert!(snap.active_transports.iter().any(|v| v == "relay-server"));
    assert!(snap
        .active_transports
        .iter()
        .any(|v| v == "connection-limits"));
}

#[tokio::test]
#[ignore = "hostile load test: set P2P_NET_TEST_RELAY_ADDR=/ip4/.../tcp/.../p2p/<relay> and run cargo test --test multi_node_hostile -- --ignored relay_reservation_spam_does_not_panic"]
async fn relay_reservation_spam_does_not_panic() {
    let Ok(relay_addr) = std::env::var("P2P_NET_TEST_RELAY_ADDR") else {
        eprintln!("skipping external relay spam test: P2P_NET_TEST_RELAY_ADDR is not set");
        return;
    };

    let mut clients = Vec::new();
    for i in 0..16u32 {
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

    tokio::time::sleep(Duration::from_secs(5)).await;

    for (handle, key_path, cache_path) in clients {
        handle.shutdown().await;
        let _ = fs::remove_file(key_path);
        let _ = fs::remove_file(cache_path);
    }
}

#[tokio::test]
#[ignore = "hostile load test: set P2P_NET_TEST_TARGET_ADDR=/ip4/.../tcp/.../p2p/<peer> and run cargo test --test multi_node_hostile -- --ignored circuit_open_close_spam_does_not_hang"]
async fn circuit_open_close_spam_does_not_hang() {
    let Ok(target_addr) = std::env::var("P2P_NET_TEST_TARGET_ADDR") else {
        eprintln!("skipping circuit/open-close spam test: P2P_NET_TEST_TARGET_ADDR is not set");
        return;
    };

    for i in 0..16u32 {
        let cfg = NodeConfig {
            bootstrap_peers: vec![target_addr.clone()],
            ..test_config(&format!("open-close-spam-{i}"))
        };
        let key_path = cfg.identity_key_path.clone();
        let cache_path = cfg.discovery.peer_cache_path.clone();
        let handle = start_node(cfg).await.expect("start dialer");
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.shutdown().await;
        let _ = fs::remove_file(key_path);
        let _ = fs::remove_file(cache_path);
    }
}

#[tokio::test]
#[ignore = "hostile soak test: run manually with cargo test --test multi_node_hostile -- --ignored long_running_soak_node_stays_responsive"]
async fn long_running_soak_node_stays_responsive() {
    let cfg = test_config("soak");
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("start node");

    tokio::time::sleep(Duration::from_secs(60)).await;
    let snap = handle.snapshot.lock().await.clone();
    handle.shutdown().await;
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(cache_path);

    assert!(snap.uptime_secs <= 120);
}

fn test_config(label: &str) -> NodeConfig {
    NodeConfig {
        identity_key_path: temp_path(&format!("{label}-key"))
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        discovery: DiscoveryConfig {
            peer_cache_path: temp_path(&format!("{label}-cache"))
                .to_string_lossy()
                .to_string(),
            ..DiscoveryConfig::default()
        },
        heartbeat_interval_secs: 1,
        ..NodeConfig::default()
    }
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("p2p-net-hostile-{prefix}-{}", PeerId::random()))
}
