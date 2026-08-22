use std::fs;
use std::path::Path;

use libp2p::PeerId;
use p2p_net::{
    app_ident_topic, app_topic_name, decode_app_message, encode_app_message, normalize_app_topic,
    validate_app_message_authentication, AppMessage, NodeMetrics, PeerInfo, PeerSource,
    MAX_APP_MESSAGE_BYTES,
};

#[test]
fn application_message_codec_round_trips_addressed_and_broadcast_messages() {
    let source = PeerId::random();
    let target = PeerId::random();

    let target_text = target.to_string();
    let addressed =
        AppMessage::addressed(7, "chat/general", source, target, b"hello peer".to_vec())
            .expect("addressed message");
    assert_eq!(
        addressed.target_peer_id.as_deref(),
        Some(target_text.as_str())
    );
    let target_peer: PeerId = target_text.parse().expect("target peer id parses");
    assert!(addressed.is_for_peer(&target_peer));
    assert!(!addressed.is_for_peer(&PeerId::random()));

    let decoded =
        decode_app_message(&encode_app_message(&addressed).expect("encode")).expect("decode");
    assert_eq!(decoded, addressed);

    let broadcast_source = PeerId::random();
    let broadcast = AppMessage::broadcast(7, "game/lobby", broadcast_source, b"hello all".to_vec())
        .expect("broadcast message");
    assert!(broadcast.target_peer_id.is_none());
    assert!(broadcast.is_for_peer(&PeerId::random()));
}

#[test]
fn app_message_authentication_binds_signed_author_and_outer_topic() {
    let author = PeerId::random();
    let other = PeerId::random();
    let message =
        AppMessage::broadcast(7, "chat/general", author, b"hello".to_vec()).expect("message");
    let topic = app_ident_topic(7, "chat/general").expect("topic").hash();

    assert!(validate_app_message_authentication(&message, &author, &topic).is_ok());
    assert!(validate_app_message_authentication(&message, &other, &topic).is_err());

    let wrong_topic = app_ident_topic(7, "chat/other")
        .expect("other topic")
        .hash();
    assert!(validate_app_message_authentication(&message, &author, &wrong_topic).is_err());
}

#[test]
fn app_topics_are_namespaced_and_validated() {
    assert_eq!(
        normalize_app_topic(" chat/general ").unwrap(),
        "chat/general"
    );
    assert_eq!(
        app_topic_name(42, "chat/general").unwrap(),
        "p2p-net/app/v2/net-42/chat/general"
    );
    assert!(normalize_app_topic("").is_err());
    assert!(normalize_app_topic("bad topic with spaces").is_err());
}

#[test]
fn app_payload_size_is_bounded() {
    let source = PeerId::random();
    let oversized = vec![0u8; MAX_APP_MESSAGE_BYTES + 1];
    assert!(AppMessage::broadcast(1, "oversized", source, oversized).is_err());
}

#[test]
fn peer_info_exposes_discovery_sources_and_capability_hints() {
    let peer = PeerId::random();
    let connected = PeerInfo::connected(peer);
    assert!(connected.connected);
    assert!(connected.has_source(PeerSource::Connected));
    assert_eq!(PeerSource::DhtProvider.as_str(), "dht_provider");
    assert_eq!(PeerSource::PublicRendezvous.as_str(), "public_rendezvous");

    let discovered = PeerInfo::discovered(
        peer,
        PeerSource::Rendezvous,
        ["/ip4/127.0.0.1/tcp/4001/p2p/example".to_string()],
    )
    .with_namespace("p2p-net/1/hydra-msg/tag-hash");
    assert!(!discovered.connected);
    assert!(discovered.has_source(PeerSource::Rendezvous));
    assert_eq!(
        discovered.namespace.as_deref(),
        Some("p2p-net/1/hydra-msg/tag-hash")
    );

    let json = serde_json::to_string(&discovered).expect("serialize peer info");
    assert!(json.contains("rendezvous"));
    assert!(json.contains("p2p-net/1/hydra-msg/tag-hash"));
}

#[test]
fn node_handle_exposes_six_app_primitives_plus_metrics_query() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let handle_rs =
        fs::read_to_string(manifest_dir.join("crates/node/handle.rs")).expect("read handle source");

    for primitive in [
        "connect_peer",
        "disconnect_peer",
        "send_message",
        "broadcast",
        "subscribe",
        "get_peers",
        "get_metrics",
    ] {
        assert!(
            handle_rs.contains(&format!("pub async fn {primitive}")),
            "NodeHandle must expose primitive: {primitive}"
        );
    }

    assert!(handle_rs.contains("AppSubscription"));
    assert!(handle_rs.contains("enum NodeCommand"));
    assert!(handle_rs.contains("NodeCommand::ConnectPeer"));
    assert!(handle_rs.contains("NodeCommand::DisconnectPeer"));
    assert!(handle_rs.contains("NodeCommand::SendMessage"));
    assert!(handle_rs.contains("NodeCommand::Broadcast"));
    assert!(handle_rs.contains("NodeCommand::Subscribe"));
    assert!(handle_rs.contains("NodeCommand::GetPeers"));
    assert!(handle_rs.contains("NodeCommand::GetMetrics"));

    let api_rs =
        fs::read_to_string(manifest_dir.join("crates/api/mod.rs")).expect("read api source");
    assert!(api_rs.contains("pub trait P2PNode"));
    assert!(api_rs.contains("fn get_metrics"));
}

#[test]
fn node_metrics_can_be_scoped_to_one_peer() {
    let peer_a = PeerId::random();
    let peer_b = PeerId::random();
    let mut metrics = NodeMetrics::default();
    metrics.bandwidth.total_bytes_sent = 300;
    metrics
        .bandwidth
        .peer_stats
        .entry(peer_a)
        .or_default()
        .bytes_sent = 100;
    metrics
        .bandwidth
        .peer_stats
        .entry(peer_b)
        .or_default()
        .bytes_sent = 200;
    metrics
        .bandwidth
        .topic_stats
        .entry("chat/general".to_string())
        .or_default()
        .bytes_sent = 300;

    let scoped = metrics.for_peer(Some(peer_a));
    assert_eq!(scoped.bandwidth.total_bytes_sent, 300);
    assert_eq!(scoped.bandwidth.peer_stats.len(), 1);
    assert_eq!(
        scoped
            .bandwidth
            .peer_stats
            .get(&peer_a)
            .expect("peer_a stats")
            .bytes_sent,
        100
    );
    assert!(!scoped.bandwidth.peer_stats.contains_key(&peer_b));
    assert!(scoped.bandwidth.topic_stats.is_empty());
}
