//! Native side of the browser direct-upgrade QA (`qa/browser/run-direct-upgrade.cjs`).
//!
//! Starts, on loopback only:
//! - a Circuit Relay node reachable over WebRTC-direct (browsers dial it) and
//!   TCP (native peers), with operator-asserted external addresses so its
//!   reservations are usable;
//! - a native peer that listens on WebRTC-direct (a direct browser→native path);
//! - a native relay-only peer with no listeners at all (reachable only through
//!   the relay; it does not speak browser `/webrtc` signaling).
//!
//! Prints one JSON line describing them, then answers line commands on stdin:
//! `stats` prints the relay's metered transit and the native peers' received
//! message counts; `quit` (or EOF) shuts everything down.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use p2p_net::{
    start_node, DhtDiscoveryConfig, DiscoveryConfig, NodeConfig, NodeHandle, NodeProfile,
    PublicBootstrapConfig, PublicIpProbeConfig, RelayServiceConfig, RendezvousConfig,
};
use tokio::io::{AsyncBufReadExt, BufReader};

const TOPIC: &str = "qa-direct-upgrade";

fn free_port(udp: bool) -> u16 {
    if udp {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .and_then(|socket| socket.local_addr())
            .map(|addr| addr.port())
            .expect("free UDP port")
    } else {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|addr| addr.port())
            .expect("free TCP port")
    }
}

fn base_config(label: &str) -> NodeConfig {
    let path = |suffix: &str| {
        std::env::temp_dir()
            .join(format!(
                "p2p-net-qa-browser-{label}-{suffix}-{}",
                std::process::id()
            ))
            .to_string_lossy()
            .to_string()
    };
    let mut cfg = NodeConfig {
        profile: NodeProfile::Full,
        identity_key_path: path("key"),
        webrtc_certificate_path: path("cert"),
        bootstrap_peers: Vec::new(),
        relay_peers: Vec::new(),
        listen_addresses: Vec::new(),
        discovery: DiscoveryConfig {
            peer_cache_path: path("cache"),
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
    };
    // The browser upgrade must not lean on LAN discovery.
    cfg.discovery.lan.enabled = false;
    cfg
}

async fn webrtc_direct_addr(node: &NodeHandle) -> String {
    loop {
        let snapshot = node.snapshot.lock().await.clone();
        if let Some(addr) = snapshot
            .local_listen_addresses
            .iter()
            .chain(snapshot.public_direct_listen_addresses.iter())
            .find(|addr| addr.contains("/webrtc-direct/certhash/"))
        {
            return addr.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn count_messages(node: &NodeHandle, counter: Arc<AtomicU64>) {
    let mut inbox = node.subscribe(TOPIC).await.expect("subscribe QA topic");
    tokio::spawn(async move {
        while inbox.recv().await.is_ok() {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tcp_port = free_port(false);
    let udp_port = free_port(true);
    let relay_tcp = format!("/ip4/127.0.0.1/tcp/{tcp_port}");
    let relay_udp = format!("/ip4/127.0.0.1/udp/{udp_port}/webrtc-direct");
    let mut relay_cfg = base_config("relay");
    relay_cfg.listen_addresses = vec![relay_tcp.clone(), relay_udp.clone()];
    relay_cfg.external_addresses = vec![relay_tcp.clone()];
    relay_cfg.relay = RelayServiceConfig {
        enabled: true,
        max_circuit_bytes: 256 * 1024 * 1024,
        max_circuit_duration_secs: 600,
        ..RelayServiceConfig::default()
    };
    let relay = start_node(relay_cfg).await?;
    // The WebRTC-direct address carries the certhash, known once listening.
    let relay_browser_addr = format!("{}/p2p/{}", webrtc_direct_addr(&relay).await, relay.peer_id);
    let relay_addr = format!("{relay_tcp}/p2p/{}", relay.peer_id);

    let webrtc_port = free_port(true);
    let mut direct_cfg = base_config("native-direct");
    direct_cfg.listen_addresses = vec![format!("/ip4/127.0.0.1/udp/{webrtc_port}/webrtc-direct")];
    let direct_peer = start_node(direct_cfg).await?;

    let mut relayed_cfg = base_config("native-relay-only");
    relayed_cfg.relay_peers = vec![relay_addr.clone()];
    relayed_cfg.reserve_configured_relays = true;
    relayed_cfg.dcutr.enabled = false;
    let relayed_peer = start_node(relayed_cfg).await?;

    let direct_received = Arc::new(AtomicU64::new(0));
    let relayed_received = Arc::new(AtomicU64::new(0));
    count_messages(&direct_peer, direct_received.clone()).await;
    count_messages(&relayed_peer, relayed_received.clone()).await;

    let webrtc_direct = format!(
        "{}/p2p/{}",
        webrtc_direct_addr(&direct_peer).await,
        direct_peer.peer_id
    );

    println!(
        "{}",
        serde_json::json!({
            "relay": { "peerId": relay.peer_id.to_string(), "addr": relay_browser_addr },
            "nativeDirect": { "peerId": direct_peer.peer_id.to_string(), "addr": webrtc_direct },
            "nativeRelayOnly": { "peerId": relayed_peer.peer_id.to_string() },
            "topic": TOPIC,
        })
    );

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match line.trim() {
            "stats" => {
                let snapshot = relay.snapshot.lock().await.clone();
                println!(
                    "{}",
                    serde_json::json!({
                        "relayBytes": snapshot.relay_bytes_forwarded,
                        "relayActiveCircuits": snapshot.relay_usage.active_circuits.len(),
                        "relayCompletedCircuits": snapshot.relay_usage.completed_circuits,
                        "relayReservations": snapshot.relay_reservations_accepted_total,
                        "relayDeniedCircuits": snapshot.relay_denied_circuits,
                        "relayServerErrors": snapshot.relay_server_errors,
                        "relayPulses": snapshot.pulses.iter().filter(|p| p.contains("relay_server")).cloned().collect::<Vec<_>>(),
                        "nativeDirectReceived": direct_received.load(Ordering::SeqCst),
                        "nativeRelayOnlyReceived": relayed_received.load(Ordering::SeqCst),
                    })
                );
            }
            "quit" => break,
            _ => println!("{{\"error\":\"unknown command\"}}"),
        }
    }

    for node in [relayed_peer, direct_peer, relay] {
        node.shutdown().await;
    }
    Ok(())
}
