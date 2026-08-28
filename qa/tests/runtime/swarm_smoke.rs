use std::time::Duration;

use libp2p::multiaddr::Protocol;
use p2p_net::{
    start_node, DhtDiscoveryConfig, DiscoveryConfig, Multiaddr, NodeConfig, NodeHandle, PeerId,
    PublicBootstrapConfig, PublicIpProbeConfig, RendezvousConfig,
};

#[tokio::test]
async fn node_start_shutdown_smoke() {
    let cfg = test_node_config("swarm-smoke");
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("start node");
    handle.shutdown().await;
    cleanup_file(&key_path);
    cleanup_file(&cache_path);
}

#[tokio::test]
async fn persistent_key_produces_same_peer_id_across_restarts() {
    let cfg = test_node_config("persistent-peer-id");
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();

    let first = start_node(cfg.clone()).await.expect("start first node");
    let first_peer = first.peer_id;
    first.shutdown().await;

    let second = start_node(cfg).await.expect("start second node");
    let second_peer = second.peer_id;
    second.shutdown().await;

    cleanup_file(&key_path);
    cleanup_file(&cache_path);
    assert_eq!(first_peer, second_peer);
}

#[tokio::test]
async fn same_lan_nodes_auto_connect_without_manual_dial_within_60s() {
    // GitHub-hosted macOS runners do not grant Local Network access to the
    // runner process, so multicast LAN discovery cannot be exercised there.
    // Keep this production-path test active everywhere else, including local
    // and self-hosted macOS runners where the OS can grant that permission.
    if github_hosted_macos_runner() {
        eprintln!(
            "skipping same-LAN multicast smoke test: GitHub-hosted macOS runners do not provide Local Network access"
        );
        return;
    }

    let network_id = 77_001;
    let lan_port = available_udp_port();
    let mut alice_cfg = test_node_config("lan-auto-alice");
    let mut bob_cfg = test_node_config("lan-auto-bob");
    for cfg in [&mut alice_cfg, &mut bob_cfg] {
        cfg.network_id = network_id;
        cfg.discovery.lan.enabled = true;
        cfg.discovery.lan.port = lan_port;
        cfg.discovery.lan.announce_interval_secs = 1;
        // Keep public infrastructure disabled for this deterministic LAN test,
        // but exercise the production auto-connect path once LAN discovery
        // supplies an authenticated compatibility-scoped candidate.
        cfg.discovery.public_bootstrap.auto_connect_discovered_peers = true;
    }

    let alice_key = alice_cfg.identity_key_path.clone();
    let alice_cache = alice_cfg.discovery.peer_cache_path.clone();
    let bob_key = bob_cfg.identity_key_path.clone();
    let bob_cache = bob_cfg.discovery.peer_cache_path.clone();

    let alice = start_node(alice_cfg).await.expect("start LAN alice node");
    let bob = start_node(bob_cfg).await.expect("start LAN bob node");

    wait_for_both_connected(&alice, bob.peer_id, &bob, alice.peer_id)
        .await
        .expect("same-LAN nodes should auto-connect without connect_peer within 60s");

    alice.shutdown().await;
    bob.shutdown().await;
    cleanup_file(&alice_key);
    cleanup_file(&alice_cache);
    cleanup_file(&bob_key);
    cleanup_file(&bob_cache);
}

#[tokio::test]
async fn native_webrtc_direct_transport_connects_two_start_node_instances_within_60s() {
    let alice_cfg = test_node_config("webrtc-direct-alice");
    let bob_cfg = test_node_config("webrtc-direct-bob");
    let alice_key = alice_cfg.identity_key_path.clone();
    let alice_cache = alice_cfg.discovery.peer_cache_path.clone();
    let bob_key = bob_cfg.identity_key_path.clone();
    let bob_cache = bob_cfg.discovery.peer_cache_path.clone();

    let alice = start_node(alice_cfg).await.expect("start alice node");
    let bob = start_node(bob_cfg).await.expect("start bob node");

    let bob_addr = wait_for_webrtc_direct_listen_addr(&bob)
        .await
        .expect("bob should expose a /webrtc-direct listen address");
    let dial_addr = with_peer_id(bob_addr, bob.peer_id);

    alice
        .connect_peer(dial_addr)
        .await
        .expect("alice should dial bob over native webrtc-direct");

    wait_for_both_connected(&alice, bob.peer_id, &bob, alice.peer_id)
        .await
        .expect("both nodes should report native webrtc-direct connection within 60s");
    let alice_metrics = alice
        .get_metrics(Some(bob.peer_id))
        .await
        .expect("alice metrics query should succeed");
    let bob_bandwidth = alice_metrics
        .bandwidth
        .peer_stats
        .get(&bob.peer_id)
        .expect("alice should track per-peer transport accounting for bob");
    assert!(bob_bandwidth.bytes_sent > 0);
    assert!(bob_bandwidth.bytes_recv > 0);
    assert!(alice_metrics.bandwidth.topic_stats.is_empty());

    alice.shutdown().await;
    bob.shutdown().await;
    cleanup_file(&alice_key);
    cleanup_file(&alice_cache);
    cleanup_file(&bob_key);
    cleanup_file(&bob_cache);
}

fn test_node_config(prefix: &str) -> NodeConfig {
    NodeConfig {
        identity_key_path: std::env::temp_dir()
            .join(format!("p2p-net-{prefix}-{}.key", libp2p::PeerId::random()))
            .to_string_lossy()
            .to_string(),
        discovery: DiscoveryConfig {
            peer_cache_path: std::env::temp_dir()
                .join(format!(
                    "p2p-net-{prefix}-{}.cache.json",
                    libp2p::PeerId::random()
                ))
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
        listen_addresses: vec![
            "/ip4/127.0.0.1/udp/0/quic-v1".to_string(),
            "/ip4/127.0.0.1/udp/0/webrtc-direct".to_string(),
            "/ip4/127.0.0.1/tcp/0".to_string(),
            "/ip4/127.0.0.1/tcp/0/ws".to_string(),
        ],
        ..NodeConfig::default()
    }
}

async fn wait_for_webrtc_direct_listen_addr(handle: &NodeHandle) -> Option<Multiaddr> {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let snapshot = handle.snapshot.lock().await;
            if let Some(addr) = snapshot
                .local_listen_addresses
                .iter()
                .chain(snapshot.public_direct_listen_addresses.iter())
                .find(|addr| addr.contains("/webrtc-direct/certhash/"))
                .and_then(|addr| addr.parse::<Multiaddr>().ok())
            {
                return Some(addr);
            }
            drop(snapshot);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .ok()
    .flatten()
}

async fn wait_for_both_connected(
    first: &NodeHandle,
    first_peer: PeerId,
    second: &NodeHandle,
    second_peer: PeerId,
) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let first_connected = peer_connected(first, first_peer).await?;
            let second_connected = peer_connected(second, second_peer).await?;
            if first_connected && second_connected {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out waiting for {first_peer} and {second_peer} to connect over native webrtc-direct"
        )
    })?
}

async fn peer_connected(handle: &NodeHandle, peer: PeerId) -> Result<bool, String> {
    let peers = handle
        .get_peers()
        .await
        .map_err(|err| format!("get_peers failed: {err}"))?;
    Ok(peers
        .iter()
        .any(|info| info.peer_id == peer.to_string() && info.connected))
}

fn with_peer_id(mut addr: Multiaddr, peer: PeerId) -> Multiaddr {
    if !addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::P2p(_)))
    {
        addr.push(Protocol::P2p(peer));
    }
    addr
}

fn available_udp_port() -> u16 {
    let socket = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind ephemeral UDP port");
    socket.local_addr().expect("local UDP address").port()
}

fn github_hosted_macos_runner() -> bool {
    cfg!(target_os = "macos")
        && std::env::var_os("GITHUB_ACTIONS").as_deref() == Some(std::ffi::OsStr::new("true"))
        && std::env::var_os("RUNNER_ENVIRONMENT").as_deref()
            == Some(std::ffi::OsStr::new("github-hosted"))
}

fn cleanup_file(path: &str) {
    let _ = std::fs::remove_file(path);
}
