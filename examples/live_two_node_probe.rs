//! Live public-network probe for two isolated p2p-net instances.
//!
//! Run with:
//! `cargo run --example live_two_node_probe`

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use libp2p::multiaddr::Protocol;
use p2p_net::{start_node, NodeConfig, NodeHandle, NodeProfile};

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut alice_cfg = probe_config("alice", nonce, 46_101, 46_102);
    let mut bob_cfg = probe_config("bob", nonce, 46_201, 46_202);
    alice_cfg.network_id = nonce as u32;
    bob_cfg.network_id = alice_cfg.network_id;

    let alice = start_node(alice_cfg.clone()).await?;
    let bob = start_node(bob_cfg.clone()).await?;
    println!("alice={}", alice.peer_id);
    println!("bob={}", bob.peer_id);

    let mut alice_pulses = HashSet::new();
    let mut bob_pulses = HashSet::new();
    let mut tried_relay_routes = HashSet::new();
    let mut last_relay_dial = None;
    let started = Instant::now();
    let deadline = started + PROBE_TIMEOUT;
    let connected = loop {
        let now = Instant::now();
        if now >= deadline {
            break false;
        }
        tokio::time::sleep((deadline - now).min(Duration::from_secs(5))).await;
        print_new_pulses("alice", &alice, &mut alice_pulses).await;
        print_new_pulses("bob", &bob, &mut bob_pulses).await;

        let alice_to_bob = peer_is_connected(&alice, bob.peer_id).await?;
        let bob_to_alice = peer_is_connected(&bob, alice.peer_id).await?;
        let alice_snapshot = alice.snapshot.lock().await.clone();
        let bob_snapshot = bob.snapshot.lock().await.clone();
        let retry_relay = last_relay_dial
            .map(|last: Instant| last.elapsed() >= Duration::from_secs(12))
            .unwrap_or(true);
        if retry_relay {
            if let Some(addr) = alice_snapshot
                .relayed_listen_addresses
                .iter()
                .filter_map(|addr| addr.parse().ok())
                .filter_map(|addr| supported_relay_addr_score(&addr).map(|score| (score, addr)))
                .filter(|(_, addr)| {
                    relay_route_key(addr)
                        .map(|route| !tried_relay_routes.contains(&route))
                        .unwrap_or(false)
                })
                .min_by_key(|(score, _)| *score)
                .map(|(_, addr)| addr)
            {
                if let Some(route) = relay_route_key(&addr) {
                    tried_relay_routes.insert(route);
                }
                last_relay_dial = Some(Instant::now());
                if let Some(addr) = build_safe_relay_dial_addr(addr, bob.peer_id, alice.peer_id) {
                    println!("probe relayed dial bob->alice addr={addr}");
                    if let Err(err) = bob.connect_peer(addr).await {
                        println!("probe relayed dial failed immediately: {err}");
                    }
                }
            }
        }
        let dcutr_successes = alice_snapshot.dcutr_successes + bob_snapshot.dcutr_successes;
        let dcutr_eligible = alice_snapshot.dcutr_upgrade_eligible_connections
            + bob_snapshot.dcutr_upgrade_eligible_connections;
        println!(
            "elapsed={}s alice_connected={} bob_connected={} alice_dht_peers={} bob_dht_peers={} alice_auto_dials={} bob_auto_dials={} alice_relays={}/{}/{} bob_relays={}/{}/{} dcutr_eligible={} dcutr_successes={}",
            started.elapsed().as_secs(),
            alice_to_bob,
            bob_to_alice,
            alice_snapshot.dht_provider_peers_discovered,
            bob_snapshot.dht_provider_peers_discovered,
            alice_snapshot.auto_connect_dial_attempts,
            bob_snapshot.auto_connect_dial_attempts,
            alice_snapshot.relay_client_reservations,
            alice_snapshot.relay_client_reservation_attempts,
            alice_snapshot.relay_client_reservation_failures,
            bob_snapshot.relay_client_reservations,
            bob_snapshot.relay_client_reservation_attempts,
            bob_snapshot.relay_client_reservation_failures,
            dcutr_eligible,
            dcutr_successes,
        );

        if alice_to_bob && bob_to_alice {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
    };

    let alice_target = alice
        .get_peers()
        .await?
        .into_iter()
        .find(|peer| peer.peer_id == bob.peer_id.to_string());
    let bob_target = bob
        .get_peers()
        .await?
        .into_iter()
        .find(|peer| peer.peer_id == alice.peer_id.to_string());
    println!("alice target: {alice_target:#?}");
    println!("bob target: {bob_target:#?}");
    alice.shutdown().await;
    bob.shutdown().await;
    cleanup(&alice_cfg);
    cleanup(&bob_cfg);

    if !connected {
        return Err("two live public-network instances did not connect before timeout".into());
    }
    println!("LIVE_TWO_NODE_RESULT=connected");
    Ok(())
}

fn probe_config(label: &str, nonce: u128, transport_port: u16, websocket_port: u16) -> NodeConfig {
    let temp = std::env::temp_dir();
    let mut cfg = NodeConfig::default();
    cfg.profile = NodeProfile::Lite;
    cfg.heartbeat_interval_secs = 5;
    cfg.identity_key_path = temp
        .join(format!("p2p-net-live-{label}-{nonce}.identity"))
        .to_string_lossy()
        .to_string();
    cfg.discovery.peer_cache_path = temp
        .join(format!("p2p-net-live-{label}-{nonce}.peers.json"))
        .to_string_lossy()
        .to_string();
    cfg.discovery.namespace.tags = vec![format!("live-dcutr-{nonce}")];
    let webrtc_port = transport_port.saturating_add(50);
    cfg.listen_addresses = vec![
        format!("/ip4/0.0.0.0/udp/{transport_port}/quic-v1"),
        format!("/ip4/0.0.0.0/udp/{webrtc_port}/webrtc-direct"),
        format!("/ip4/0.0.0.0/tcp/{transport_port}"),
        format!("/ip4/0.0.0.0/tcp/{websocket_port}/ws"),
    ];
    cfg
}

async fn peer_is_connected(
    node: &NodeHandle,
    peer: libp2p::PeerId,
) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(node
        .get_peers()
        .await?
        .iter()
        .any(|known| known.peer_id == peer.to_string() && known.connected))
}

async fn print_new_pulses(label: &str, node: &NodeHandle, seen: &mut HashSet<String>) {
    let snapshot = node.snapshot.lock().await;
    for pulse in &snapshot.pulses {
        if seen.insert(pulse.clone()) {
            println!("{label}: {pulse}");
        }
    }
}

fn cleanup(cfg: &NodeConfig) {
    let _ = std::fs::remove_file(&cfg.identity_key_path);
    let _ = std::fs::remove_file(&cfg.discovery.peer_cache_path);
}

fn supported_relay_addr_score(addr: &libp2p::Multiaddr) -> Option<u8> {
    if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::WebTransport))
    {
        return None;
    }
    if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Quic | Protocol::QuicV1))
    {
        Some(0)
    } else if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::WebRTCDirect | Protocol::P2pWebRtcDirect))
    {
        Some(1)
    } else if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Tcp(_)))
    {
        Some(2)
    } else {
        None
    }
}

fn relay_route_key(addr: &libp2p::Multiaddr) -> Option<String> {
    let mut relay = None;
    let mut transport = None;
    for protocol in addr.iter() {
        match protocol {
            Protocol::P2p(peer) if relay.is_none() => relay = Some(peer),
            Protocol::P2pCircuit => break,
            Protocol::Quic | Protocol::QuicV1 => transport = Some("quic"),
            Protocol::WebRTCDirect | Protocol::P2pWebRtcDirect => transport = Some("webrtc"),
            Protocol::Tcp(_) => transport = Some("tcp"),
            _ => {}
        }
    }
    Some(format!("{}:{}", relay?, transport?))
}

fn build_safe_relay_dial_addr(
    mut addr: libp2p::Multiaddr,
    local_peer: libp2p::PeerId,
    destination: libp2p::PeerId,
) -> Option<libp2p::Multiaddr> {
    if destination == local_peer {
        return None;
    }

    let mut saw_circuit = false;
    let mut target_after_circuit = None;
    for protocol in addr.iter() {
        match protocol {
            Protocol::P2pCircuit => saw_circuit = true,
            Protocol::P2p(peer) if saw_circuit => target_after_circuit = Some(peer),
            _ => {}
        }
    }

    if !saw_circuit {
        return None;
    }

    match target_after_circuit {
        Some(existing) if existing == destination => Some(addr),
        Some(_) => None,
        None => {
            addr.push(Protocol::P2p(destination));
            Some(addr)
        }
    }
}
