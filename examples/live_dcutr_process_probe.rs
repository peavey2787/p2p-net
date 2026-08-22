//! Two-process live DCUtR acceptance probe.
//!
//! `cargo run --release --example live_dcutr_process_probe`

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use libp2p::multiaddr::Protocol;
use p2p_net::{start_node, Multiaddr, NodeConfig, NodeProfile, PeerId, PublicBootstrapConfig};
use serde::{Deserialize, Serialize};

const TIMEOUT: Duration = Duration::from_secs(60);
const STATUS_MAX_AGE: Duration = Duration::from_secs(5);
const VERIFIED_PUBLIC_RELAY: &str =
    "/ip4/162.19.78.231/udp/4001/quic-v1/p2p/12D3KooWA5z81YbRuKMfdxKhn1MYxLvGq9XrLT5Lwo3EHLjQgLZH";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProcessStatus {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    updated_unix_ms: u128,
    peer_id: String,
    relayed_addresses: Vec<String>,
    relay_reservation_attempts: usize,
    relay_reservation_failures: usize,
    relay_selected: Vec<String>,
    relay_events: Vec<String>,
    connected_target: bool,
    target_relay_seen: bool,
    target_direct_after_relay: bool,
    target_supports_dcutr: Option<bool>,
    dcutr_enabled: bool,
    dcutr_eligible: usize,
    dcutr_attempts: usize,
    dcutr_failures: usize,
    dcutr_successes: usize,
    dcutr_events: Vec<String>,
    target_dial_errors: Vec<String>,
    direct_candidates: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("--child") {
        return run_child(&args).await;
    }
    run_parent().await
}

async fn run_parent() -> Result<(), Box<dyn std::error::Error>> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let session = std::env::temp_dir().join(format!("p2p-net-dcutr-live-{nonce}"));
    std::fs::create_dir_all(&session)?;
    let current_exe = std::env::current_exe()?;
    let network_id = nonce as u32;
    let tag = format!("live-dcutr-process-{nonce}");

    let started = Instant::now();
    let mut alice = spawn_child(
        &current_exe,
        "alice",
        &session,
        network_id,
        &tag,
        47_101,
        47_102,
    )?;
    let mut bob = spawn_child(
        &current_exe,
        "bob",
        &session,
        network_id,
        &tag,
        47_201,
        47_202,
    )?;

    let mut last_dcutr_events = HashSet::new();
    let mut last_target_dial_errors = HashSet::new();
    let success = loop {
        let remaining = TIMEOUT.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break false;
        }
        tokio::time::sleep(remaining.min(Duration::from_secs(2))).await;
        let alice_status =
            read_fresh_status(&session.join("alice.status.json"), &tag, STATUS_MAX_AGE);
        let bob_status = read_fresh_status(&session.join("bob.status.json"), &tag, STATUS_MAX_AGE);
        if let (Some(alice_status), Some(bob_status)) = (&alice_status, &bob_status) {
            println!(
                "elapsed={}s alice_relays={} bob_relays={} connected={}/{} target_relay={}/{} target_direct={}/{} target_dcutr={:?}/{:?} enabled={}/{} dcutr_eligible={} attempts={} failures={} successes={}",
                started.elapsed().as_secs(),
                alice_status.relayed_addresses.len(),
                bob_status.relayed_addresses.len(),
                alice_status.connected_target,
                bob_status.connected_target,
                alice_status.target_relay_seen,
                bob_status.target_relay_seen,
                alice_status.target_direct_after_relay,
                bob_status.target_direct_after_relay,
                alice_status.target_supports_dcutr,
                bob_status.target_supports_dcutr,
                alice_status.dcutr_enabled,
                bob_status.dcutr_enabled,
                alice_status.dcutr_eligible + bob_status.dcutr_eligible,
                alice_status.dcutr_attempts + bob_status.dcutr_attempts,
                alice_status.dcutr_failures + bob_status.dcutr_failures,
                alice_status.dcutr_successes + bob_status.dcutr_successes,
            );
            for event in alice_status
                .dcutr_events
                .iter()
                .chain(&bob_status.dcutr_events)
            {
                if last_dcutr_events.insert(event.clone()) {
                    println!("dcutr_detail={event}");
                }
            }
            for error in alice_status
                .target_dial_errors
                .iter()
                .chain(&bob_status.target_dial_errors)
            {
                if last_target_dial_errors.insert(error.clone()) {
                    println!("target_dial_detail={error}");
                }
            }
            if started.elapsed().as_secs() <= 8 {
                println!(
                    "direct_candidates alice={:?} bob={:?}",
                    alice_status.direct_candidates, bob_status.direct_candidates
                );
            }
            if alice_status.connected_target
                && bob_status.connected_target
                && alice_status.target_relay_seen
                && bob_status.target_relay_seen
                && alice_status.target_direct_after_relay
                && bob_status.target_direct_after_relay
            {
                break true;
            }
        }
        if started.elapsed() >= TIMEOUT {
            break false;
        }
    };

    stop_child(&mut alice);
    stop_child(&mut bob);
    cleanup_session(&session);
    if !success {
        return Err("LIVE_DCUTR_RESULT=failed timeout=60s".into());
    }
    println!("LIVE_DCUTR_RESULT=connected_and_hole_punched");
    Ok(())
}

fn spawn_child(
    exe: &Path,
    role: &str,
    session: &Path,
    network_id: u32,
    tag: &str,
    transport_port: u16,
    websocket_port: u16,
) -> Result<Child, std::io::Error> {
    Command::new(exe)
        .args([
            "--child",
            role,
            &session.to_string_lossy(),
            &network_id.to_string(),
            tag,
            &transport_port.to_string(),
            &websocket_port.to_string(),
        ])
        .spawn()
}

async fn run_child(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let role = args.get(2).ok_or("missing child role")?;
    let session = PathBuf::from(args.get(3).ok_or("missing session path")?);
    let network_id = args.get(4).ok_or("missing network id")?.parse::<u32>()?;
    let tag = args.get(5).ok_or("missing discovery tag")?;
    let transport_port = args
        .get(6)
        .ok_or("missing transport port")?
        .parse::<u16>()?;
    let websocket_port = args
        .get(7)
        .ok_or("missing websocket port")?
        .parse::<u16>()?;
    let direct_smoke_mode = args.get(8).is_some();
    let direct_smoke_base = args.get(8).filter(|arg| arg.as_str() != "-").cloned();
    let other_role = if role == "alice" { "bob" } else { "alice" };
    let own_status_path = session.join(format!("{role}.status.json"));
    let other_status_path = session.join(format!("{other_role}.status.json"));

    let mut cfg = NodeConfig {
        profile: NodeProfile::Lite,
        network_id,
        heartbeat_interval_secs: 5,
        identity_key_path: session
            .join(format!("{role}.identity"))
            .to_string_lossy()
            .to_string(),
        ..NodeConfig::default()
    };
    cfg.discovery.peer_cache_path = session
        .join(format!("{role}.peers.json"))
        .to_string_lossy()
        .to_string();
    cfg.discovery.namespace.tags = vec![tag.clone()];
    cfg.discovery.dht.enabled = false;
    cfg.discovery.public_bootstrap.auto_connect_discovered_peers = false;
    cfg.discovery.public_bootstrap.relay_peers = vec![VERIFIED_PUBLIC_RELAY.to_string()];
    cfg.discovery.relay_discovery.use_cached_relays = false;
    cfg.discovery.relay_discovery.use_rendezvous_relays = false;
    cfg.discovery.relay_discovery.use_dht_relays = false;
    cfg.discovery.relay_discovery.min_reservations = 1;
    cfg.discovery.relay_discovery.max_reservations = 1;
    if direct_smoke_mode {
        cfg.discovery.public_bootstrap = PublicBootstrapConfig::private_infrastructure_only();
        cfg.discovery.dht.enabled = false;
        cfg.discovery.relay_discovery.enabled = false;
        cfg.relay_peers.clear();
    }
    let webrtc_port = transport_port.saturating_add(50);
    cfg.listen_addresses = vec![
        format!("/ip4/0.0.0.0/udp/{transport_port}/quic-v1"),
        format!("/ip4/0.0.0.0/udp/{webrtc_port}/webrtc-direct"),
        format!("/ip4/0.0.0.0/tcp/{transport_port}"),
        format!("/ip4/0.0.0.0/tcp/{websocket_port}/ws"),
    ];

    let node = start_node(cfg).await?;
    let mut tried_routes = HashMap::new();
    let mut last_dial = None;
    let mut processed_connection_pulses = HashSet::new();
    let mut accumulated_dcutr_events = HashSet::new();
    let mut accumulated_target_dial_errors = HashSet::new();
    let mut target_relay_seen = false;
    let mut target_direct_after_relay = false;
    let mut last_direct_smoke_dial = None;
    let started = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let other_status = read_fresh_status(&other_status_path, tag, STATUS_MAX_AGE);
        let target_peer = other_status
            .as_ref()
            .and_then(|status| status.peer_id.parse::<PeerId>().ok());
        let peers = node.get_peers().await?;
        let connected_target = target_peer
            .as_ref()
            .map(|target| {
                peers
                    .iter()
                    .any(|peer| peer.peer_id == target.to_string() && peer.connected)
            })
            .unwrap_or(false);
        let target_supports_dcutr = target_peer.as_ref().and_then(|target| {
            peers
                .iter()
                .find(|peer| peer.peer_id == target.to_string())
                .and_then(|peer| peer.supports_dcutr)
        });
        let snapshot = node.snapshot.lock().await.clone();
        let target_label = target_peer.map(|peer| peer.to_string());
        if let Some(target) = &target_label {
            for pulse in &snapshot.pulses {
                if !pulse.contains("connection endpoint")
                    || !pulse.contains(&format!("peer={target}"))
                    || !processed_connection_pulses.insert(pulse.clone())
                {
                    continue;
                }
                if pulse.contains("relayed=true") {
                    target_relay_seen = true;
                } else if pulse.contains("relayed=false") && target_relay_seen {
                    target_direct_after_relay = true;
                }
            }
            for pulse in &snapshot.pulses {
                if pulse.contains("dcutr event")
                    && pulse.contains(target)
                    && pulse.contains("result: Ok")
                    && target_relay_seen
                {
                    target_direct_after_relay = true;
                }
            }
            accumulated_dcutr_events.extend(
                snapshot
                    .pulses
                    .iter()
                    .filter(|pulse| {
                        pulse.contains("dcutr event")
                            || (pulse.contains("connection endpoint")
                                && pulse.contains(&format!("peer={target}")))
                    })
                    .cloned(),
            );
            accumulated_target_dial_errors.extend(
                snapshot
                    .pulses
                    .iter()
                    .filter(|pulse| {
                        pulse.contains("outgoing connection error") && pulse.contains(target)
                    })
                    .cloned(),
            );
        }
        let mut direct_candidates = snapshot.public_direct_listen_addresses.clone();
        direct_candidates.extend(snapshot.local_listen_addresses.clone());
        direct_candidates.sort();
        direct_candidates.dedup();
        let mut dcutr_events = accumulated_dcutr_events.iter().cloned().collect::<Vec<_>>();
        dcutr_events.sort();
        let relay_events = snapshot
            .pulses
            .iter()
            .filter(|pulse| pulse.contains("relay"))
            .cloned()
            .collect();
        let mut target_dial_errors = accumulated_target_dial_errors
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        target_dial_errors.sort();
        let status = ProcessStatus {
            session_id: tag.to_string(),
            updated_unix_ms: now_unix_ms(),
            peer_id: node.peer_id.to_string(),
            relayed_addresses: snapshot.relayed_listen_addresses.clone(),
            relay_reservation_attempts: snapshot.relay_client_reservation_attempts,
            relay_reservation_failures: snapshot.relay_client_reservation_failures,
            relay_selected: snapshot.relay_discovery_selected_relays.clone(),
            relay_events,
            connected_target,
            target_relay_seen,
            target_direct_after_relay,
            target_supports_dcutr,
            dcutr_enabled: snapshot.dcutr_enabled,
            dcutr_eligible: snapshot.dcutr_upgrade_eligible_connections,
            dcutr_attempts: snapshot.dcutr_attempts,
            dcutr_failures: snapshot.dcutr_failures,
            dcutr_successes: snapshot.dcutr_successes,
            dcutr_events,
            target_dial_errors,
            direct_candidates,
        };
        std::fs::write(&own_status_path, serde_json::to_vec(&status)?)?;

        let direct_smoke_retry = last_direct_smoke_dial
            .map(|last: Instant| last.elapsed() >= Duration::from_secs(3))
            .unwrap_or(true);
        if !connected_target && direct_smoke_retry {
            if let (Some(base), Some(destination)) = (&direct_smoke_base, target_peer) {
                let mut addr = base.parse::<Multiaddr>()?;
                addr.push(Protocol::P2p(destination));
                last_direct_smoke_dial = Some(Instant::now());
                println!("{role}: direct smoke dial {addr}");
                let _ = node.connect_peer(addr).await;
            }
        }

        let retry = last_dial
            .map(|last: Instant| last.elapsed() >= Duration::from_secs(8))
            .unwrap_or(true);
        // Establish exactly one relayed connection. Two simultaneous circuits
        // start competing DCUtR state machines that reuse the same QUIC port.
        if role == "alice" && !direct_smoke_mode && !connected_target && retry {
            if let Some(other) = other_status {
                let destination = other.peer_id.parse::<PeerId>()?;
                if destination != node.peer_id {
                    if let Some(candidate) = select_untried_relay(
                        &other.relayed_addresses,
                        &status.relayed_addresses,
                        &tried_routes,
                    ) {
                        if let Some(route) = relay_route_key(&candidate) {
                            let attempts = tried_routes.entry(route).or_insert(0_u8);
                            *attempts = attempts.saturating_add(1);
                        }
                        if let Some(addr) =
                            build_safe_relay_dial_addr(candidate, node.peer_id, destination)
                        {
                            last_dial = Some(Instant::now());
                            println!("{role}: relayed dial {addr}");
                            let _ = tokio::time::timeout(
                                Duration::from_secs(8),
                                node.connect_peer(addr),
                            )
                            .await;
                        }
                    }
                }
            }
        }
        if status.connected_target && (status.target_direct_after_relay || direct_smoke_mode) {
            tokio::time::sleep(Duration::from_secs(2)).await;
            node.shutdown().await;
            return Ok(());
        }
        if started.elapsed() >= TIMEOUT + Duration::from_secs(5) {
            node.shutdown().await;
            return Ok(());
        }
    }
}

fn select_untried_relay(
    addresses: &[String],
    local_addresses: &[String],
    tried: &HashMap<String, u8>,
) -> Option<Multiaddr> {
    let local_relays = local_addresses
        .iter()
        .filter_map(|addr| addr.parse::<Multiaddr>().ok())
        .filter_map(|addr| relay_peer_from_addr(&addr))
        .collect::<HashSet<_>>();
    addresses
        .iter()
        .filter_map(|addr| addr.parse::<Multiaddr>().ok())
        .filter_map(|addr| {
            let score = supported_relay_addr_score(&addr)?;
            let route = relay_route_key(&addr)?;
            let shared = relay_peer_from_addr(&addr)
                .map(|relay| !local_relays.contains(&relay))
                .unwrap_or(true);
            let attempts = tried.get(&route).copied().unwrap_or(0);
            (attempts < 1).then_some((shared, attempts, score, addr))
        })
        .min_by_key(|(shared, attempts, score, _)| (*shared, *attempts, *score))
        .map(|(_, _, _, addr)| addr)
}

fn supported_relay_addr_score(addr: &Multiaddr) -> Option<u8> {
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

fn relay_route_key(addr: &Multiaddr) -> Option<String> {
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

fn relay_peer_from_addr(addr: &Multiaddr) -> Option<PeerId> {
    for protocol in addr.iter() {
        match protocol {
            Protocol::P2p(peer) => return Some(peer),
            Protocol::P2pCircuit => return None,
            _ => {}
        }
    }
    None
}

fn build_safe_relay_dial_addr(
    mut addr: Multiaddr,
    local_peer: PeerId,
    destination: PeerId,
) -> Option<Multiaddr> {
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

fn read_fresh_status(path: &Path, session_id: &str, max_age: Duration) -> Option<ProcessStatus> {
    let bytes = std::fs::read(path).ok()?;
    let status: ProcessStatus = serde_json::from_slice(&bytes).ok()?;
    if status.session_id != session_id {
        return None;
    }
    if status.updated_unix_ms == 0 {
        return None;
    }
    if now_unix_ms().saturating_sub(status.updated_unix_ms) > max_age.as_millis() {
        return None;
    }
    Some(status)
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn cleanup_session(session: &Path) {
    for name in [
        "alice.status.json",
        "bob.status.json",
        "alice.identity",
        "bob.identity",
        "alice.peers.json",
        "bob.peers.json",
    ] {
        let _ = std::fs::remove_file(session.join(name));
    }
    let _ = std::fs::remove_dir(session);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_relay_builder_appends_missing_destination_once() {
        let local = PeerId::random();
        let relay = PeerId::random();
        let destination = PeerId::random();
        let addr: Multiaddr = format!("/ip4/203.0.113.1/tcp/4001/p2p/{relay}/p2p-circuit")
            .parse()
            .expect("relay addr");

        let built = build_safe_relay_dial_addr(addr, local, destination).expect("built");

        assert_eq!(
            built.to_string(),
            format!("/ip4/203.0.113.1/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{destination}")
        );
    }

    #[test]
    fn safe_relay_builder_keeps_existing_destination() {
        let local = PeerId::random();
        let relay = PeerId::random();
        let destination = PeerId::random();
        let addr: Multiaddr =
            format!("/ip4/203.0.113.1/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{destination}")
                .parse()
                .expect("relay addr");

        let built = build_safe_relay_dial_addr(addr.clone(), local, destination).expect("built");

        assert_eq!(built, addr);
    }

    #[test]
    fn safe_relay_builder_rejects_local_peer_and_mismatched_target() {
        let local = PeerId::random();
        let relay = PeerId::random();
        let other = PeerId::random();
        let addr: Multiaddr = format!("/ip4/203.0.113.1/tcp/4001/p2p/{relay}/p2p-circuit")
            .parse()
            .expect("relay addr");
        assert!(build_safe_relay_dial_addr(addr, local, local).is_none());

        let target = PeerId::random();
        let mismatched: Multiaddr =
            format!("/ip4/203.0.113.1/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{other}")
                .parse()
                .expect("relay addr");
        assert!(build_safe_relay_dial_addr(mismatched, local, target).is_none());
    }
}
