//! Cross-machine public-network probe for one isolated p2p-net instance.
//!
//! Run one copy on each machine with a different `P2P_LIVE_PROBE_ROLE`. Each
//! process has 60 seconds to discover and connect to the requested number of
//! application peers through the normal production planner. Set
//! `P2P_LIVE_PROBE_EXPECT_PEERS=2` for a three-node stability run. Network ID
//! and discovery namespace stay at the exact production defaults unless
//! `P2P_LIVE_PROBE_TAG` supplies a shared private test tag. Only local
//! identity/cache paths and listen ports are isolated so concurrent probe
//! processes cannot collide. Set `P2P_LIVE_PROBE_DISABLE_LAN=1` when the run
//! must prove public discovery and relay/direct connectivity without LAN help.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use p2p_net::{start_node, NodeConfig, NodeProfile};

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
// One node can satisfy the discovery gate almost a full timeout before another.
// Keep successful nodes alive long enough for the slowest node to complete its
// own hold window without manufacturing a test-only disconnect.
const HOLD_SHUTDOWN_GRACE: Duration = Duration::from_secs(70);

macro_rules! probe_log {
    ($($arg:tt)*) => {{
        println!($($arg)*);
        let mut stdout = std::io::stdout();
        let _ = std::io::Write::flush(&mut stdout);
    }};
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let role = required_env("P2P_LIVE_PROBE_ROLE")?;
    let expected_peers = expected_peer_count()?;
    let nonce = std::process::id();
    let cfg = probe_config(&role, nonce);
    assert_eq!(cfg.network_id, NodeConfig::default().network_id);
    if std::env::var_os("P2P_LIVE_PROBE_TAG").is_none() {
        assert_eq!(
            cfg.discovery.namespace,
            NodeConfig::default().discovery.namespace
        );
    }
    let network_id = cfg.network_id;
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
        let application_peers = connected_application_peers(&node).await?;
        probe_log!(
            "role={role} elapsed={}s application_peers={}/{} app_peer_ids={:?} app_swarm={} all_swarm={} peer_book={} dht_announced={} dht_queries={}/{} dht_peers={} auto_dials={} relay_reservations={}/{} dcutr_eligible={} dcutr_successes={} last_app_dial_error={:?}",
            started.elapsed().as_secs(),
            application_peers.len(),
            expected_peers,
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
        if snapshot.application_peer_connections >= expected_peers {
            break true;
        }
    };

    if !connected {
        node.shutdown().await;
        cleanup(&cfg);
        return Err(format!(
            "single live node did not connect to {expected_peers} application peer(s) before timeout"
        )
        .into());
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
            print_new_pulses(&role, &node, &mut seen_pulses).await;
            let snapshot = node.snapshot.lock().await.clone();
            let application_peers = connected_application_peers(&node).await?;
            probe_log!(
                "role={role} hold_elapsed={}s application_peers={}/{} app_peer_ids={:?} app_swarm={} all_swarm={} peer_book={} auto_dials={} dcutr_eligible={} dcutr_successes={}",
                hold_started.elapsed().as_secs(),
                application_peers.len(),
                expected_peers,
                application_peers,
                snapshot.application_peer_connections,
                snapshot.all_swarm_connections,
                snapshot.peer_book_known_peers,
                snapshot.auto_connect_dial_attempts,
                snapshot.dcutr_upgrade_eligible_connections,
                snapshot.dcutr_successes,
            );
            if snapshot.application_peer_connections < expected_peers {
                let disconnected_at = disconnected_since.get_or_insert_with(Instant::now);
                if disconnected_at.elapsed() >= PROBE_TIMEOUT {
                    node.shutdown().await;
                    cleanup(&cfg);
                    return Err(format!(
                        "expected {expected_peers} application peers were not restored within the 60-second reconnect window"
                    )
                    .into());
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

    // Keep an early-finishing side alive through the maximum discovery skew so
    // every other node can record its own final successful sample.
    if held_connection {
        tokio::time::sleep(HOLD_SHUTDOWN_GRACE).await;
    }

    node.shutdown().await;
    cleanup(&cfg);
    probe_log!("LIVE_SINGLE_NODE_RESULT=connected role={role} expected_peers={expected_peers}");
    Ok(())
}

fn probe_config(role: &str, nonce: u32) -> NodeConfig {
    let temp = std::env::temp_dir();
    let role_hash = stable_role_hash(role);
    let transport_port = 47_000u16.saturating_add((role_hash % 500) as u16);
    let mut cfg = NodeConfig {
        profile: NodeProfile::Full,
        heartbeat_interval_secs: heartbeat_interval_secs(),
        identity_key_path: temp
            .join(format!("p2p-net-live-{role}-{nonce}.identity"))
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec![
            format!("/ip4/0.0.0.0/udp/{transport_port}/quic-v1"),
            format!(
                "/ip4/0.0.0.0/udp/{}/webrtc-direct",
                transport_port.saturating_add(500)
            ),
            format!("/ip4/0.0.0.0/tcp/{transport_port}"),
            format!("/ip4/0.0.0.0/tcp/{}/ws", transport_port.saturating_add(1)),
        ],
        ..NodeConfig::default()
    };
    if let Ok(tag) = std::env::var("P2P_LIVE_PROBE_TAG") {
        let tag = tag.trim();
        if !tag.is_empty() {
            cfg.discovery.namespace.tags = vec![tag.to_string()];
        }
    }
    if env_flag("P2P_LIVE_PROBE_DISABLE_LAN") {
        cfg.discovery.lan.enabled = false;
    }
    cfg.discovery.peer_cache_path = temp
        .join(format!("p2p-net-live-{role}-{nonce}.peers.json"))
        .to_string_lossy()
        .to_string();
    cfg
}

fn stable_role_hash(value: &str) -> u32 {
    let hash = blake3::hash(value.as_bytes());
    u32::from_le_bytes(hash.as_bytes()[..4].try_into().expect("four bytes"))
}

fn required_env(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| format!("{name} must be set").into())
}

fn hold_duration() -> Option<Duration> {
    std::env::var("P2P_LIVE_PROBE_HOLD_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

fn heartbeat_interval_secs() -> u64 {
    std::env::var("P2P_LIVE_PROBE_HEARTBEAT_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(5)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn expected_peer_count() -> Result<usize, Box<dyn std::error::Error>> {
    let count = std::env::var("P2P_LIVE_PROBE_EXPECT_PEERS")
        .unwrap_or_else(|_| "1".to_string())
        .parse::<usize>()?;
    if count == 0 {
        return Err("P2P_LIVE_PROBE_EXPECT_PEERS must be at least 1".into());
    }
    Ok(count)
}

async fn connected_application_peers(
    node: &p2p_net::NodeHandle,
) -> Result<Vec<String>, p2p_net::NetError> {
    let mut peers = node
        .get_peers()
        .await?
        .into_iter()
        .filter(|peer| peer.connected && peer.namespace.is_some())
        .map(|peer| peer.peer_id)
        .collect::<Vec<_>>();
    peers.sort();
    Ok(peers)
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
