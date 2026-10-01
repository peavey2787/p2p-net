//! Relay metering through real nodes: real application traffic crosses a
//! real p2p-net relay node, and its per-circuit events, `RelayState`
//! snapshot and Prometheus export all report the bytes counted on the relay
//! data plane. No byte value is injected anywhere in this test.

use std::time::Duration;

use libp2p::{Multiaddr, PeerId};
use p2p_net::{
    snapshot_to_prometheus_metrics, start_node, DhtDiscoveryConfig, DiscoveryConfig, NodeConfig,
    NodeEvent, NodeEventSubscription, NodeHandle, NodeProfile, PublicBootstrapConfig,
    PublicIpProbeConfig, RelayServiceConfig, RendezvousConfig,
};

const STEP: Duration = Duration::from_secs(30);
const TOPIC: &str = "relay-metering";
const MESSAGES: usize = 12;
const MESSAGE_BYTES: usize = 8 * 1024;

#[tokio::test]
async fn real_relayed_traffic_is_metered_into_events_snapshot_and_metrics() {
    // A relay must advertise a reachable address in its reservations; on
    // loopback that is an operator-asserted external address.
    let port = free_tcp_port();
    let relay = start_node(relay_config("metering-relay", port))
        .await
        .expect("start relay node");
    let relay_addr = wait_for_tcp_dial_addr(&relay).await;
    let mut relay_events = relay.subscribe_events();

    // Dial-only clients with DCUtR off: the only path between them is the
    // relay circuit, so every application byte must cross the relay.
    let dst = start_node(client_config("metering-dst", &relay_addr))
        .await
        .expect("start destination");
    let src = start_node(client_config("metering-src", &relay_addr))
        .await
        .expect("start source");
    // Both clients hold reservations (so both are connected to the relay)
    // before the relayed dial.
    wait_for_reservations(&relay, 2).await;

    let mut dst_inbox = dst.subscribe(TOPIC).await.expect("subscribe destination");
    let mut src_inbox = src.subscribe(TOPIC).await.expect("subscribe source");
    let circuit: Multiaddr = format!("{relay_addr}/p2p-circuit/p2p/{}", dst.peer_id)
        .parse()
        .unwrap();
    src.connect_peer(circuit).await.expect("relayed connect");
    let (circuit_id, src_peer, dst_peer) =
        tokio::time::timeout(STEP, wait_opened(&mut relay_events))
            .await
            .expect("relay opens a circuit");
    assert_eq!(src_peer, src.peer_id.to_string());
    assert_eq!(dst_peer, dst.peer_id.to_string());

    // Gossipsub needs a moment to learn the peer's topic subscription.
    wait_subscribed(&src, dst.peer_id).await;
    wait_subscribed(&dst, src.peer_id).await;
    for i in 0..MESSAGES {
        src.send_message(dst.peer_id, TOPIC, vec![i as u8; MESSAGE_BYTES])
            .await
            .expect("send src->dst");
        dst.send_message(src.peer_id, TOPIC, vec![i as u8; MESSAGE_BYTES / 2])
            .await
            .expect("send dst->src");
    }
    for _ in 0..MESSAGES {
        recv(&mut dst_inbox).await;
        recv(&mut src_inbox).await;
    }

    // Live metering is visible while the circuit is still open.
    let live = wait_for_live_usage(&relay, circuit_id).await;
    assert!(live.bytes_src_to_dst >= (MESSAGES * MESSAGE_BYTES) as u64);
    assert!(live.bytes_dst_to_src >= (MESSAGES * MESSAGE_BYTES / 2) as u64);

    src.disconnect_peer(dst.peer_id)
        .await
        .expect("close relayed path");
    let closed = wait_closed(&mut relay_events, circuit_id).await;
    let NodeEvent::RelayCircuitClosed {
        bytes_src_to_dst,
        bytes_dst_to_src,
        src_peer_id,
        dst_peer_id,
        ..
    } = closed.clone()
    else {
        unreachable!()
    };
    assert_eq!(src_peer_id, src.peer_id.to_string());
    assert_eq!(dst_peer_id, dst.peer_id.to_string());
    assert!(bytes_src_to_dst >= live.bytes_src_to_dst, "monotonic");
    assert!(bytes_dst_to_src >= live.bytes_dst_to_src, "monotonic");

    // Snapshot and Prometheus are derived from the same per-circuit totals.
    let snapshot = wait_for_completed(&relay).await;
    let billed: u64 = snapshot
        .relay_usage
        .recent_closed_circuits
        .iter()
        .map(|usage| usage.total_transit_bytes())
        .sum();
    assert_eq!(snapshot.relay_bytes_forwarded, billed);
    let record = snapshot
        .relay_usage
        .recent_closed_circuits
        .iter()
        .find(|usage| usage.circuit_id == circuit_id)
        .expect("closed circuit is in the snapshot");
    assert_eq!(record.bytes_src_to_dst, bytes_src_to_dst);
    assert_eq!(record.bytes_dst_to_src, bytes_dst_to_src);
    let metrics = snapshot_to_prometheus_metrics(&snapshot);
    assert!(metrics.contains(&format!(
        "p2p_relay_bytes_forwarded {}\n",
        snapshot.relay_bytes_forwarded
    )));
    assert!(metrics.contains(&format!(
        "p2p_relay_bytes_src_to_dst {}\n",
        snapshot.relay_usage.bytes_src_to_dst
    )));
    assert!(metrics.contains(&format!(
        "p2p_relay_bytes_dst_to_src {}\n",
        snapshot.relay_usage.bytes_dst_to_src
    )));
    assert!(snapshot.relay_bytes_forwarded >= (MESSAGES * MESSAGE_BYTES * 3 / 2) as u64);

    for node in [src, dst, relay] {
        node.shutdown().await;
    }
}

async fn wait_subscribed(node: &NodeHandle, peer: PeerId) {
    tokio::time::timeout(STEP, async {
        // A zero-length probe is enough to learn whether the topic has peers.
        while node.send_message(peer, TOPIC, Vec::new()).await.is_err() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("peer subscribed to the test topic");
}

async fn recv(inbox: &mut p2p_net::AppSubscription) {
    tokio::time::timeout(STEP, inbox.recv())
        .await
        .expect("message within step")
        .expect("subscription open");
}

async fn wait_opened(events: &mut NodeEventSubscription) -> (u64, String, String) {
    loop {
        let event = events.recv().await.expect("relay events open");
        if let NodeEvent::RelayCircuitOpened {
            circuit_id,
            src_peer_id,
            dst_peer_id,
        } = event
        {
            return (circuit_id, src_peer_id, dst_peer_id);
        }
    }
}

async fn wait_closed(events: &mut NodeEventSubscription, id: u64) -> NodeEvent {
    loop {
        let event = tokio::time::timeout(STEP, events.recv())
            .await
            .expect("relay event within step")
            .expect("relay events open");
        if matches!(&event, NodeEvent::RelayCircuitClosed { circuit_id, .. } if *circuit_id == id) {
            return event;
        }
    }
}

async fn wait_for_live_usage(relay: &NodeHandle, id: u64) -> p2p_net::RelayCircuitUsage {
    tokio::time::timeout(STEP, async {
        loop {
            let snapshot = relay.snapshot.lock().await.clone();
            if let Some(usage) = snapshot
                .relay_usage
                .active_circuits
                .iter()
                .find(|usage| usage.circuit_id == id && usage.bytes_src_to_dst > 0)
            {
                return usage.clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("live circuit usage reaches the snapshot")
}

async fn wait_for_completed(relay: &NodeHandle) -> p2p_net::NodeSnapshot {
    tokio::time::timeout(STEP, async {
        loop {
            let snapshot = relay.snapshot.lock().await.clone();
            if snapshot.relay_usage.active_circuits.is_empty()
                && snapshot.relay_usage.completed_circuits > 0
            {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("all relay circuits complete")
}

async fn wait_for_reservations(handle: &NodeHandle, count: usize) {
    tokio::time::timeout(STEP, async {
        while handle
            .snapshot
            .lock()
            .await
            .relay_reservations_accepted_total
            < count
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("destination reserves a relay slot");
}

async fn wait_for_tcp_dial_addr(handle: &NodeHandle) -> String {
    tokio::time::timeout(STEP, async {
        loop {
            let snapshot = handle.snapshot.lock().await;
            if let Some(addr) = snapshot
                .local_listen_addresses
                .iter()
                .find(|addr| addr.contains("/tcp/") && !addr.contains("/ws"))
            {
                return format!("{addr}/p2p/{}", handle.peer_id);
            }
            drop(snapshot);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("relay TCP listen address")
}

fn free_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("free loopback port")
}

fn relay_config(label: &str, port: u16) -> NodeConfig {
    NodeConfig {
        external_addresses: vec![format!("/ip4/127.0.0.1/tcp/{port}")],
        relay: RelayServiceConfig {
            enabled: true,
            max_circuit_bytes: 64 * 1024 * 1024,
            max_circuit_duration_secs: 300,
            ..RelayServiceConfig::default()
        },
        listen_addresses: vec![format!("/ip4/127.0.0.1/tcp/{port}")],
        ..base_config(label)
    }
}

fn client_config(label: &str, relay_addr: &str) -> NodeConfig {
    let mut cfg = NodeConfig {
        relay_peers: vec![relay_addr.to_string()],
        reserve_configured_relays: true,
        listen_addresses: Vec::new(),
        ..base_config(label)
    };
    cfg.dcutr.enabled = false;
    cfg
}

fn base_config(label: &str) -> NodeConfig {
    let path = |suffix: &str| {
        std::env::temp_dir()
            .join(format!(
                "p2p-net-metering-{label}-{suffix}-{}",
                PeerId::random()
            ))
            .to_string_lossy()
            .to_string()
    };
    NodeConfig {
        profile: NodeProfile::Full,
        identity_key_path: path("key"),
        bootstrap_peers: Vec::new(),
        relay_peers: Vec::new(),
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
    }
}
