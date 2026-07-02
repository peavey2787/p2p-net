//! Live public-network probe for two isolated p2p-net instances.
//!
//! Run with:
//! `cargo run --example live_two_node_probe`

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use p2p_net::{start_node, NodeConfig, NodeHandle, NodeProfile};

const PROBE_TIMEOUT: Duration = Duration::from_secs(180);

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
    let started = Instant::now();
    let connected = loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        print_new_pulses("alice", &alice, &mut alice_pulses).await;
        print_new_pulses("bob", &bob, &mut bob_pulses).await;

        let alice_to_bob = peer_is_connected(&alice, bob.peer_id).await?;
        let bob_to_alice = peer_is_connected(&bob, alice.peer_id).await?;
        let alice_snapshot = alice.snapshot.lock().await.clone();
        let bob_snapshot = bob.snapshot.lock().await.clone();
        println!(
            "elapsed={}s alice_connected={} bob_connected={} alice_dht_peers={} bob_dht_peers={} alice_auto_dials={} bob_auto_dials={} alice_relays={}/{}/{} bob_relays={}/{}/{}",
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
        );

        if alice_to_bob && bob_to_alice {
            break true;
        }
        if started.elapsed() >= PROBE_TIMEOUT {
            break false;
        }
    };

    let alice_peers = alice.get_peers().await?;
    let bob_peers = bob.get_peers().await?;
    println!("alice peer book: {alice_peers:#?}");
    println!("bob peer book: {bob_peers:#?}");
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
    cfg.profile = NodeProfile::Full;
    cfg.heartbeat_interval_secs = 5;
    cfg.identity_key_path = temp
        .join(format!("p2p-net-live-{label}-{nonce}.identity"))
        .to_string_lossy()
        .to_string();
    cfg.discovery.peer_cache_path = temp
        .join(format!("p2p-net-live-{label}-{nonce}.peers.json"))
        .to_string_lossy()
        .to_string();
    cfg.discovery.namespace.tags = vec![format!("live-two-node-{nonce}")];
    cfg.listen_addresses = vec![
        format!("/ip4/0.0.0.0/udp/{transport_port}/quic-v1"),
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
