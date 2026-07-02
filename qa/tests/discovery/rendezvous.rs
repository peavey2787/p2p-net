use std::fs;

use libp2p::{Multiaddr, PeerId};
use p2p_net::connectivity::rendezvous::RendezvousActionPlan;
use p2p_net::{snapshot_to_json, start_node, DiscoveryConfig, NodeConfig, RendezvousConfig};

#[test]
fn rendezvous_config_defaults_to_client_on_server_off_and_validates() {
    let cfg = RendezvousConfig::default();
    assert!(cfg.client_enabled);
    assert!(!cfg.server_enabled);
    cfg.validate().expect("default rendezvous config validates");
}

#[test]
fn rendezvous_config_rejects_bad_resource_limits() {
    let bad = RendezvousConfig {
        server_min_ttl_secs: 10,
        server_max_ttl_secs: 5,
        ..RendezvousConfig::default()
    };
    assert!(bad.validate().is_err());

    let bad = RendezvousConfig {
        server_max_registrations_per_peer: 8,
        server_max_registrations_total: 4,
        ..RendezvousConfig::default()
    };
    assert!(bad.validate().is_err());
}

#[test]
fn rendezvous_peer_config_requires_p2p_peer_id() {
    let bad = NodeConfig {
        discovery: DiscoveryConfig {
            rendezvous_peers: vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
            rendezvous: RendezvousConfig {
                client_enabled: true,
                ..RendezvousConfig::default()
            },
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(bad.validate().is_err());

    let good = NodeConfig {
        discovery: DiscoveryConfig {
            rendezvous_peers: vec![p2p_addr(4001).to_string()],
            rendezvous: RendezvousConfig {
                client_enabled: true,
                ..RendezvousConfig::default()
            },
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    good.validate().expect("rendezvous peer config validates");
}

#[test]
fn rendezvous_action_plan_defaults_empty() {
    let plan = RendezvousActionPlan::default();
    assert_eq!(plan.register_attempts, 0);
    assert_eq!(plan.discover_attempts, 0);
    assert!(plan.errors.is_empty());
}

#[tokio::test]
async fn rendezvous_server_can_be_enabled_without_client() {
    let cfg = NodeConfig {
        identity_key_path: temp_path("rzv-server-key").to_string_lossy().to_string(),
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        discovery: DiscoveryConfig {
            peer_cache_path: temp_path("rzv-server-cache").to_string_lossy().to_string(),
            rendezvous: RendezvousConfig {
                client_enabled: false,
                server_enabled: true,
                ..RendezvousConfig::default()
            },
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("rendezvous server starts");
    let snap = handle.snapshot.lock().await.clone();
    let json = snapshot_to_json(&snap);
    handle.shutdown().await;
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(cache_path);

    assert!(snap.rendezvous_server_enabled);
    assert!(!snap.rendezvous_client_enabled);
    assert_eq!(json["rendezvous_server_enabled"], true);
    assert_eq!(json["rendezvous_client_enabled"], false);
}

#[tokio::test]
async fn rendezvous_client_reports_attempts_for_configured_peer() {
    let cfg = NodeConfig {
        identity_key_path: temp_path("rzv-client-key").to_string_lossy().to_string(),
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        discovery: DiscoveryConfig {
            peer_cache_path: temp_path("rzv-client-cache").to_string_lossy().to_string(),
            rendezvous_peers: vec![p2p_addr(4100).to_string()],
            rendezvous: RendezvousConfig {
                client_enabled: true,
                register: false,
                discover: true,
                ..RendezvousConfig::default()
            },
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg).await.expect("rendezvous client starts");
    let snap = handle.snapshot.lock().await.clone();
    handle.shutdown().await;
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(cache_path);

    assert!(snap.rendezvous_client_enabled);
    assert_eq!(snap.rendezvous_discover_attempts, 1);
}

fn p2p_addr(port: u16) -> Multiaddr {
    let peer = PeerId::random();
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer}")
        .parse()
        .unwrap()
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("p2p-net-{prefix}-{}", PeerId::random()))
}

#[test]
fn rendezvous_discovery_uses_connection_planner_auto_dial() {
    let stack_discovery = fs::read_to_string("crates/stack/discovery.rs")
        .expect("read stack discovery module");
    let rendezvous_events = fs::read_to_string("crates/node/events/rendezvous.rs")
        .expect("read node rendezvous events module");

    assert!(
        !stack_discovery.contains("let mut dialed_peer"),
        "rendezvous discovery must not keep the old first-address direct-dial path"
    );
    assert!(
        rendezvous_events.contains("auto_dial_peer_from_book"),
        "rendezvous discovery should use the shared auto-dial planner"
    );
    assert!(
        rendezvous_events.contains("ctx.pending_connections"),
        "rendezvous auto-dial should use pending connection-plan dedupe"
    );
    assert!(
        rendezvous_events.contains("auto_connect_discovered_peers"),
        "rendezvous auto-dial must remain policy-gated"
    );
    assert!(
        rendezvous_events.contains("PeerSource::PublicRendezvous"),
        "public rendezvous discoveries should stay source-accounted separately"
    );
}
