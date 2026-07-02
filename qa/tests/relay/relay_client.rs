use libp2p::{Multiaddr, PeerId};
use p2p_net::connectivity::relay::{is_p2p_circuit_addr, relay_peer_id, relay_reservation_addr};
use p2p_net::NodeConfig;

#[test]
fn default_config_reserves_configured_relays() {
    let cfg = NodeConfig::default();
    assert!(cfg.reserve_configured_relays);
    assert!(cfg.relay_peers.is_empty());
}

#[test]
fn configured_relay_peer_yields_reservation_listen_addr() {
    let relay = PeerId::random();
    let relay_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay}")
        .parse()
        .unwrap();

    let listen_addr = relay_reservation_addr(&relay_addr).expect("reservation addr");
    assert_eq!(relay_peer_id(&listen_addr), Some(relay));
    assert!(is_p2p_circuit_addr(&listen_addr));
    assert_eq!(
        listen_addr.to_string(),
        format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay}/p2p-circuit")
    );
}

#[test]
fn full_relay_target_addr_extracts_relay_peer_not_target_peer() {
    let relay = PeerId::random();
    let target = PeerId::random();
    let target_addr: Multiaddr =
        format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{target}")
            .parse()
            .unwrap();

    assert_eq!(relay_peer_id(&target_addr), Some(relay));
    assert!(is_p2p_circuit_addr(&target_addr));
}

#[test]
fn relay_peer_config_requires_peer_id() {
    let missing_peer_id = NodeConfig {
        relay_peers: vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
        ..NodeConfig::default()
    };
    assert!(missing_peer_id.validate().is_err());

    let relay = PeerId::random();
    let valid = NodeConfig {
        relay_peers: vec![format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay}")],
        ..NodeConfig::default()
    };
    valid.validate().expect("relay config validates");
}
