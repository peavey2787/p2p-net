//! Cross-machine public-network probe for one isolated p2p-net instance.
//!
//! Run one copy on each machine with the same `P2P_LIVE_PROBE_NONCE` and a
//! different `P2P_LIVE_PROBE_ROLE`. Each process has 60 seconds to discover
//! and connect to the other through the normal production planner.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use p2p_net::{start_node, NodeConfig, NodeProfile};

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const HOLD_SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

macro_rules! probe_log {
    ($($arg:tt)*) => {{
        println!($($arg)*);
        let mut stdout = std::io::stdout();
        let _ = std::io::Write::flush(&mut stdout);
    }};
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let nonce = required_env("P2P_LIVE_PROBE_NONCE")?;
    let role = required_env("P2P_LIVE_PROBE_ROLE")?;
    let network_id = stable_network_id(&nonce);
    let cfg = probe_config(&role, &nonce, network_id);
    let node = start_node(cfg.clone()).await?;

    probe_log!(
        "LIVE_SINGLE_NODE_STARTED role={role} peer={} network_id={network_id}",
        node.peer_id
    );

    let started = Instant::now();
    let mut seen_pulses = HashSet::new();
    let connected = loop {
        let now = Instant::now();
        if now >= started + PROBE_TIMEOUT {
            break false;
        }
        tokio::time::sleep((started + PROBE_TIMEOUT - now).min(Duration::from_secs(5))).await;
        print_new_pulses(&role, &node, &mut seen_pulses).await;
        let snapshot = node.snapshot.lock().await.clone();
        let application_peers = node
            .get_peers()
            .await?
            .into_iter()
            .filter(|peer| peer.connected && peer.namespace.is_some())
            .count();
        probe_log!(
            "role={role} elapsed={}s application_peers={} app_swarm={} all_swarm={} peer_book={} dht_announced={} dht_queries={}/{} dht_peers={} auto_dials={} relay_reservations={}/{} dcutr_eligible={} dcutr_successes={} last_app_dial_error={:?}",
            started.elapsed().as_secs(),
            application_peers,
            snapshot.application_peer_connections,
            snapshot.all_swarm_connections,
            snapshot.peer_book_known_peers,
            snapshot.dht_provider_namespaces_announced,
            snapshot.dht_provider_queries,
            snapshot.dht_provider_queries_finished,
            snapshot.dht_provider_peers_discovered,
            snapshot.auto_connect_dial_attempts,
            snapshot.relay_client_reservations,
            snapshot.relay_client_reservation_attempts,
            snapshot.dcutr_upgrade_eligible_connections,
            snapshot.dcutr_successes,
            snapshot.last_application_dial_error,
        );
        if application_peers > 0 {
            break true;
        }
    };

    if !connected {
        node.shutdown().await;
        cleanup(&cfg);
        return Err(
            "single live node did not connect to an application peer before timeout".into(),
        );
    }

    let held_connection = if let Some(hold) = hold_duration() {
        let hold_started = Instant::now();
        let mut disconnected_since = None;
        while hold_started.elapsed() < hold || disconnected_since.is_some() {
            let until_hold_end = hold.saturating_sub(hold_started.elapsed());
            tokio::time::sleep(
                until_hold_end
                    .min(Duration::from_secs(10))
                    .max(Duration::from_secs(1)),
            )
            .await;
            let snapshot = node.snapshot.lock().await.clone();
            let application_peers = node
                .get_peers()
                .await?
                .into_iter()
                .filter(|peer| peer.connected && peer.namespace.is_some())
                .count();
            probe_log!(
                "role={role} hold_elapsed={}s application_peers={} app_swarm={} all_swarm={} peer_book={} auto_dials={} dcutr_eligible={} dcutr_successes={}",
                hold_started.elapsed().as_secs(),
                application_peers,
                snapshot.application_peer_connections,
                snapshot.all_swarm_connections,
                snapshot.peer_book_known_peers,
                snapshot.auto_connect_dial_attempts,
                snapshot.dcutr_upgrade_eligible_connections,
                snapshot.dcutr_successes,
            );
            if application_peers == 0 {
                let disconnected_at = disconnected_since.get_or_insert_with(Instant::now);
                if disconnected_at.elapsed() >= PROBE_TIMEOUT {
                    node.shutdown().await;
                    cleanup(&cfg);
                    return Err(
                        "application peer was not restored within the 60-second reconnect window"
                            .into(),
                    );
                }
            } else if let Some(disconnected_at) = disconnected_since.take() {
                probe_log!(
                    "role={role} application_peer_reconnected_after={}s",
                    disconnected_at.elapsed().as_secs()
                );
            }
        }
        true
    } else {
        false
    };

    // Both cross-machine processes normally reach their hold deadline within
    // one polling interval of each other. Keep the completed side alive long
    // enough for the other side to record its own final successful sample.
    if held_connection {
        tokio::time::sleep(HOLD_SHUTDOWN_GRACE).await;
    }

    node.shutdown().await;
    cleanup(&cfg);
    probe_log!("LIVE_SINGLE_NODE_RESULT=connected role={role}");
    Ok(())
}

fn probe_config(role: &str, nonce: &str, network_id: u32) -> NodeConfig {
    let temp = std::env::temp_dir();
    let role_hash = stable_network_id(role);
    let transport_port = 47_000u16.saturating_add((role_hash % 500) as u16);
    let mut cfg = NodeConfig::default();
    cfg.profile = NodeProfile::Full;
    cfg.network_id = network_id;
    cfg.heartbeat_interval_secs = 5;
    cfg.identity_key_path = temp
        .join(format!("p2p-net-live-{role}-{nonce}.identity"))
        .to_string_lossy()
        .to_string();
    cfg.discovery.peer_cache_path = temp
        .join(format!("p2p-net-live-{role}-{nonce}.peers.json"))
        .to_string_lossy()
        .to_string();
    cfg.discovery.namespace.tags = vec![format!("live-cross-platform-{nonce}")];
    cfg.listen_addresses = vec![
        format!("/ip4/0.0.0.0/udp/{transport_port}/quic-v1"),
        format!(
            "/ip4/0.0.0.0/udp/{}/webrtc-direct",
            transport_port.saturating_add(500)
        ),
        format!("/ip4/0.0.0.0/tcp/{transport_port}"),
        format!("/ip4/0.0.0.0/tcp/{}/ws", transport_port.saturating_add(1)),
    ];
    cfg
}

fn stable_network_id(value: &str) -> u32 {
    let hash = blake3::hash(value.as_bytes());
    u32::from_le_bytes(hash.as_bytes()[..4].try_into().expect("four bytes"))
}

fn required_env(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| format!("{name} must be set to the shared probe value").into())
}

fn hold_duration() -> Option<Duration> {
    std::env::var("P2P_LIVE_PROBE_HOLD_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

fn cleanup(cfg: &NodeConfig) {
    let _ = std::fs::remove_file(&cfg.identity_key_path);
    let _ = std::fs::remove_file(&cfg.discovery.peer_cache_path);
}

async fn print_new_pulses(role: &str, node: &p2p_net::NodeHandle, seen: &mut HashSet<String>) {
    let snapshot = node.snapshot.lock().await;
    for pulse in &snapshot.pulses {
        if seen.insert(pulse.clone()) {
            probe_log!("{role}: {pulse}");
        }
    }
}
