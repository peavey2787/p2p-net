use std::fs;
use std::path::Path;

use libp2p::PeerId;
use p2p_net::{
    app_topic_name, decode_app_message, encode_app_message, normalize_app_topic, AppMessage,
    MAX_APP_MESSAGE_BYTES,
};

#[test]
fn application_message_codec_round_trips_addressed_and_broadcast_messages() {
    let source = PeerId::random();
    let target = PeerId::random();

    let target_text = target.to_string();
    let addressed = AppMessage::addressed(
        7,
        "chat/general",
        source,
        target,
        b"hello peer".to_vec(),
    )
    .expect("addressed message");
    assert_eq!(addressed.target_peer_id.as_deref(), Some(target_text.as_str()));
    let target_peer: PeerId = target_text.parse().expect("target peer id parses");
    assert!(addressed.is_for_peer(&target_peer));
    assert!(!addressed.is_for_peer(&PeerId::random()));

    let decoded = decode_app_message(&encode_app_message(&addressed).expect("encode"))
        .expect("decode");
    assert_eq!(decoded, addressed);

    let broadcast_source = PeerId::random();
    let broadcast = AppMessage::broadcast(7, "game/lobby", broadcast_source, b"hello all".to_vec())
        .expect("broadcast message");
    assert!(broadcast.target_peer_id.is_none());
    assert!(broadcast.is_for_peer(&PeerId::random()));
}

#[test]
fn app_topics_are_namespaced_and_validated() {
    assert_eq!(normalize_app_topic(" chat/general ").unwrap(), "chat/general");
    assert_eq!(
        app_topic_name(42, "chat/general").unwrap(),
        "p2p-net/app/v1/net-42/chat/general"
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
fn node_handle_exposes_exact_six_general_purpose_primitives() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let handle_rs = fs::read_to_string(manifest_dir.join("crates/node/handle.rs"))
        .expect("read handle source");

    for primitive in [
        "connect_peer",
        "disconnect_peer",
        "send_message",
        "broadcast",
        "subscribe",
        "get_peers",
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
}
