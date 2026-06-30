use p2p_net::{start_node, NodeConfig};

#[tokio::test]
async fn node_start_shutdown_smoke() {
    let cfg = test_node_config("swarm-smoke");
    let key_path = cfg.identity_key_path.clone();
    let handle = start_node(cfg).await.expect("start node");
    handle.shutdown().await;
    let _ = std::fs::remove_file(key_path);
}

#[tokio::test]
async fn persistent_key_produces_same_peer_id_across_restarts() {
    let cfg = test_node_config("persistent-peer-id");
    let key_path = cfg.identity_key_path.clone();

    let first = start_node(cfg.clone()).await.expect("start first node");
    let first_peer = first.peer_id;
    first.shutdown().await;

    let second = start_node(cfg).await.expect("start second node");
    let second_peer = second.peer_id;
    second.shutdown().await;

    let _ = std::fs::remove_file(key_path);
    assert_eq!(first_peer, second_peer);
}

fn test_node_config(prefix: &str) -> NodeConfig {
    NodeConfig {
        identity_key_path: std::env::temp_dir()
            .join(format!("p2p-net-{prefix}-{}.key", libp2p::PeerId::random()))
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec![
            "/ip4/127.0.0.1/udp/0/quic-v1".to_string(),
            "/ip4/127.0.0.1/tcp/0".to_string(),
            "/ip4/127.0.0.1/tcp/0/ws".to_string(),
        ],
        ..NodeConfig::default()
    }
}
