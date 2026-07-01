use p2p_net::{verify_heartbeat, HeartbeatEnvelope};

#[test]
fn valid_heartbeat_accepts() {
    let peer = libp2p::PeerId::random();
    let env = HeartbeatEnvelope::new(peer);
    assert!(verify_heartbeat(peer, &env, env.timestamp_ns));
}

#[test]
fn spoofed_peer_id_rejects() {
    let source = libp2p::PeerId::random();
    let claimed = libp2p::PeerId::random();
    let env = HeartbeatEnvelope::new(claimed);
    assert!(!verify_heartbeat(source, &env, env.timestamp_ns));
}

#[test]
fn zero_entropy_rejects() {
    let peer = libp2p::PeerId::random();
    let mut env = HeartbeatEnvelope::new(peer);
    env.entropy = vec![0u8; 32];
    env.nonce_hex = blake3::hash(&env.entropy).to_hex().to_string();
    assert!(!verify_heartbeat(peer, &env, env.timestamp_ns));
}
