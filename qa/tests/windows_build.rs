#[cfg(all(windows, target_arch = "x86_64"))]
#[tokio::test]
async fn windows_tokio_boot_smoke() {
    let cfg = p2p_net::NodeConfig {
        identity_key_path: std::env::temp_dir()
            .join(format!("p2p-net-windows-{}.key", libp2p::PeerId::random()))
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec![
            "/ip4/127.0.0.1/udp/0/quic-v1".to_string(),
            "/ip4/127.0.0.1/tcp/0".to_string(),
            "/ip4/127.0.0.1/tcp/0/ws".to_string(),
        ],
        ..p2p_net::NodeConfig::default()
    };
    let key_path = cfg.identity_key_path.clone();
    let handle = p2p_net::start_node(cfg).await.expect("start node");
    handle.shutdown().await;
    let _ = std::fs::remove_file(key_path);
}
