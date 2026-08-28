//! Live zero-manual-dial probe for two isolated default p2p-net instances.
//!
//! The only per-instance overrides are identity/cache paths and listen ports so
//! two nodes can coexist in one test process. Network ID, discovery namespace,
//! public fallback, DHT, relay discovery, LAN discovery, DCUtR, and application
//! compatibility all use production defaults. There are deliberately zero
//! manual peer-dial API calls in this probe.
//!
//! Run with:
//! `cargo run --release --example live_two_node_probe`
//!
//! Set `P2P_LIVE_PROBE_DISABLE_LAN=1` to exercise only public DHT/relay/DCUtR
//! fallback while preserving the same default network/application namespace.

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use p2p_net::{start_node, NodeConfig, NodeHandle};

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

macro_rules! probe_log {
    ($($arg:tt)*) => {{
        println!($($arg)*);
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }};
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut alice_cfg = probe_config("alice", nonce, 46_101, 46_102);
    let mut bob_cfg = probe_config("bob", nonce, 46_201, 46_202);
    if env_true("P2P_LIVE_PROBE_DISABLE_LAN") {
        alice_cfg.discovery.lan.enabled = false;
        bob_cfg.discovery.lan.enabled = false;
    }

    assert_eq!(alice_cfg.network_id, NodeConfig::default().network_id);
    assert_eq!(bob_cfg.network_id, NodeConfig::default().network_id);
    assert!(alice_cfg.discovery.namespace.tags.is_empty());
    assert!(bob_cfg.discovery.namespace.tags.is_empty());

    let alice = start_node(alice_cfg.clone()).await?;
    let bob = start_node(bob_cfg.clone()).await?;
    probe_log!("alice={}", alice.peer_id);
    probe_log!("bob={}", bob.peer_id);

    let mut alice_pulses = HashSet::new();
    let mut bob_pulses = HashSet::new();
    let started = Instant::now();
    let connected = tokio::time::timeout(PROBE_TIMEOUT, async {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            print_new_pulses("alice", &alice, &mut alice_pulses).await;
            print_new_pulses("bob", &bob, &mut bob_pulses).await;

            let alice_to_bob = peer_is_application_connected(&alice, bob.peer_id).await?;
            let bob_to_alice = peer_is_application_connected(&bob, alice.peer_id).await?;
            let alice_snapshot = alice.snapshot.lock().await.clone();
            let bob_snapshot = bob.snapshot.lock().await.clone();
            probe_log!(
                "elapsed={}s alice_app={} bob_app={} alice_dht={}/{} bob_dht={}/{} alice_known={} bob_known={} alice_relays={} bob_relays={} dcutr_eligible={} dcutr_successes={}",
                started.elapsed().as_secs(),
                alice_to_bob,
                bob_to_alice,
                alice_snapshot.dht_provider_queries,
                alice_snapshot.dht_provider_queries_finished,
                bob_snapshot.dht_provider_queries,
                bob_snapshot.dht_provider_queries_finished,
                alice_snapshot.peer_book_known_peers,
                bob_snapshot.peer_book_known_peers,
                alice_snapshot.relay_client_reservations,
                bob_snapshot.relay_client_reservations,
                alice_snapshot.dcutr_upgrade_eligible_connections
                    + bob_snapshot.dcutr_upgrade_eligible_connections,
                alice_snapshot.dcutr_successes + bob_snapshot.dcutr_successes,
            );
            if alice_to_bob && bob_to_alice {
                return Ok::<(), Box<dyn std::error::Error>>(());
            }
        }
    })
    .await;

    let result = match connected {
        Ok(result) => result,
        Err(_) => Err("two default instances did not auto-connect within 60 seconds".into()),
    };

    let alice_snapshot = alice.snapshot.lock().await.clone();
    let bob_snapshot = bob.snapshot.lock().await.clone();
    probe_log!(
        "final alice_app={} bob_app={} alice_infra={} bob_infra={} alice_relays={} bob_relays={} dcutr_successes={}",
        alice_snapshot.application_peer_connections,
        bob_snapshot.application_peer_connections,
        alice_snapshot.infrastructure_peer_connections,
        bob_snapshot.infrastructure_peer_connections,
        alice_snapshot.relay_client_reservations,
        bob_snapshot.relay_client_reservations,
        alice_snapshot.dcutr_successes + bob_snapshot.dcutr_successes,
    );

    alice.shutdown().await;
    bob.shutdown().await;
    cleanup(&alice_cfg);
    cleanup(&bob_cfg);
    result?;
    probe_log!("LIVE_TWO_NODE_RESULT=auto_connected");
    Ok(())
}

fn probe_config(label: &str, nonce: u128, transport_port: u16, websocket_port: u16) -> NodeConfig {
    let temp = std::env::temp_dir();
    let webrtc_port = transport_port.saturating_add(50);
    let mut cfg = NodeConfig {
        identity_key_path: temp
            .join(format!("p2p-net-live-{label}-{nonce}.identity"))
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec![
            format!("/ip4/0.0.0.0/udp/{transport_port}/quic-v1"),
            format!("/ip4/0.0.0.0/udp/{webrtc_port}/webrtc-direct"),
            format!("/ip4/0.0.0.0/tcp/{transport_port}"),
            format!("/ip4/0.0.0.0/tcp/{websocket_port}/ws"),
        ],
        ..NodeConfig::default()
    };
    cfg.discovery.peer_cache_path = temp
        .join(format!("p2p-net-live-{label}-{nonce}.peers.json"))
        .to_string_lossy()
        .to_string();
    cfg
}

async fn peer_is_application_connected(
    node: &NodeHandle,
    peer: libp2p::PeerId,
) -> Result<bool, Box<dyn std::error::Error>> {
    let namespaces = node.snapshot.lock().await.discovery_namespaces.clone();
    Ok(node.get_peers().await?.iter().any(|known| {
        known.peer_id == peer.to_string()
            && known.connected
            && known
                .namespace
                .as_ref()
                .is_some_and(|namespace| namespaces.contains(namespace))
    }))
}

async fn print_new_pulses(label: &str, node: &NodeHandle, seen: &mut HashSet<String>) {
    let snapshot = node.snapshot.lock().await;
    for pulse in &snapshot.pulses {
        if seen.insert(pulse.clone()) {
            probe_log!("{label}: {pulse}");
        }
    }
}

fn cleanup(cfg: &NodeConfig) {
    let _ = std::fs::remove_file(&cfg.identity_key_path);
    let _ = std::fs::remove_file(&cfg.discovery.peer_cache_path);
}

fn env_true(name: &str) -> bool {
    std::env::var(name)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}
