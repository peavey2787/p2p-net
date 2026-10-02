//! A relay reservation is re-requested after its listener closes.
//!
//! The Circuit Relay v2 client only closes the `/p2p-circuit` listener when a
//! reservation fails or the relay connection is lost; nothing re-requests it.
//! Here a real relay node goes away and comes back on the same identity and
//! port, and the client must hold a reservation on it again without any help.

use std::time::Duration;

use libp2p::PeerId;
use p2p_net::{
    start_node, DhtDiscoveryConfig, DiscoveryConfig, NodeConfig, NodeHandle, NodeProfile,
    PublicBootstrapConfig, PublicIpProbeConfig, RelayServiceConfig, RendezvousConfig,
};

const STEP: Duration = Duration::from_secs(30);

#[tokio::test]
async fn reservation_is_re_requested_after_the_relay_comes_back() {
    let port = free_tcp_port();
    let relay_key = temp_path("relay-key");
    let relay = start_node(relay_config(port, &relay_key))
        .await
        .expect("start relay");
    let relay_addr = format!("/ip4/127.0.0.1/tcp/{port}/p2p/{}", relay.peer_id);

    let client = start_node(client_config(&relay_addr))
        .await
        .expect("start client");
    wait_for_accepted(&relay).await;

    relay.shutdown().await;
    wait_for_pulse(&client, "relay_client reservation closed").await;

    // Same identity and port: the client's configured relay address is valid
    // again, and only a re-request can produce a new reservation.
    let relay = start_node(relay_config(port, &relay_key))
        .await
        .expect("restart relay");
    wait_for_accepted(&relay).await;
    wait_for_pulse(&client, "relay_client reservation re-requested").await;

    client.shutdown().await;
    relay.shutdown().await;
}

async fn wait_for_accepted(relay: &NodeHandle) {
    tokio::time::timeout(STEP, async {
        while relay
            .snapshot
            .lock()
            .await
            .relay_reservations_accepted_total
            == 0
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("relay accepts the client's reservation");
}

async fn wait_for_pulse(node: &NodeHandle, prefix: &str) {
    tokio::time::timeout(STEP, async {
        loop {
            if node
                .snapshot
                .lock()
                .await
                .pulses
                .iter()
                .any(|line| line.starts_with(prefix))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("pulse `{prefix}`"));
}

fn free_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("free loopback port")
}

fn temp_path(suffix: &str) -> String {
    std::env::temp_dir()
        .join(format!(
            "p2p-net-reservation-retry-{suffix}-{}",
            PeerId::random()
        ))
        .to_string_lossy()
        .to_string()
}

fn relay_config(port: u16, key_path: &str) -> NodeConfig {
    NodeConfig {
        identity_key_path: key_path.to_string(),
        external_addresses: vec![format!("/ip4/127.0.0.1/tcp/{port}")],
        listen_addresses: vec![format!("/ip4/127.0.0.1/tcp/{port}")],
        relay: RelayServiceConfig {
            enabled: true,
            ..RelayServiceConfig::default()
        },
        ..base_config()
    }
}

fn client_config(relay_addr: &str) -> NodeConfig {
    let mut cfg = NodeConfig {
        relay_peers: vec![relay_addr.to_string()],
        reserve_configured_relays: true,
        ..base_config()
    };
    cfg.dcutr.enabled = false;
    cfg
}

fn base_config() -> NodeConfig {
    NodeConfig {
        profile: NodeProfile::Full,
        identity_key_path: temp_path("key"),
        bootstrap_peers: Vec::new(),
        relay_peers: Vec::new(),
        listen_addresses: Vec::new(),
        discovery: DiscoveryConfig {
            peer_cache_path: temp_path("cache"),
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
