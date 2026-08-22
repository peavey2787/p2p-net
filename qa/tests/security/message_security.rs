use p2p_net::{
    encode_heartbeat_wire, validate_heartbeat_wire, verify_heartbeat_with_config, HeartbeatEnvelope,
    HeartbeatReplayCache, HeartbeatValidationDecision, MessageSecurityConfig,
};

const NS_PER_SEC: u64 = 1_000_000_000;

#[test]
fn invalid_heartbeat_is_rejected() {
    let peer = libp2p::PeerId::random();
    let cfg = MessageSecurityConfig::default();
    let mut cache = HeartbeatReplayCache::new(&cfg);
    let mut env = HeartbeatEnvelope::new(peer);
    env.entropy = vec![0u8; 32];
    env.nonce_hex = blake3::hash(&env.entropy).to_hex().to_string();
    let data = encode_heartbeat_wire(&env).unwrap();

    let result = validate_heartbeat_wire(peer, &data, env.timestamp_ns, &cfg, &mut cache);
    assert_eq!(result.decision, HeartbeatValidationDecision::Reject);
}

#[test]
fn spoofed_peer_id_heartbeat_is_rejected() {
    let source = libp2p::PeerId::random();
    let claimed = libp2p::PeerId::random();
    let cfg = MessageSecurityConfig::default();
    let mut cache = HeartbeatReplayCache::new(&cfg);
    let env = HeartbeatEnvelope::new(claimed);
    let data = encode_heartbeat_wire(&env).unwrap();

    let result = validate_heartbeat_wire(source, &data, env.timestamp_ns, &cfg, &mut cache);
    assert_eq!(result.decision, HeartbeatValidationDecision::Reject);
}

#[test]
fn old_timestamp_heartbeat_is_rejected() {
    let peer = libp2p::PeerId::random();
    let cfg = MessageSecurityConfig {
        max_heartbeat_age_secs: 60,
        ..MessageSecurityConfig::default()
    };
    let mut env = HeartbeatEnvelope::new(peer);
    let now = 1_000 * NS_PER_SEC;
    env.timestamp_ns = now - 61 * NS_PER_SEC;

    assert!(!verify_heartbeat_with_config(peer, &env, now, &cfg));
}

#[test]
fn future_timestamp_beyond_skew_is_rejected() {
    let peer = libp2p::PeerId::random();
    let cfg = MessageSecurityConfig {
        max_heartbeat_age_secs: 60,
        max_heartbeat_future_skew_secs: 5,
        ..MessageSecurityConfig::default()
    };
    let mut env = HeartbeatEnvelope::new(peer);
    let now = 1_000 * NS_PER_SEC;
    env.timestamp_ns = now + 6 * NS_PER_SEC;

    assert!(!verify_heartbeat_with_config(peer, &env, now, &cfg));
}

#[test]
fn duplicate_heartbeat_is_ignored() {
    let peer = libp2p::PeerId::random();
    let cfg = MessageSecurityConfig::default();
    let mut cache = HeartbeatReplayCache::new(&cfg);
    let env = HeartbeatEnvelope::new(peer);
    let data = encode_heartbeat_wire(&env).unwrap();

    let first = validate_heartbeat_wire(peer, &data, env.timestamp_ns, &cfg, &mut cache);
    let second = validate_heartbeat_wire(peer, &data, env.timestamp_ns, &cfg, &mut cache);

    assert_eq!(first.decision, HeartbeatValidationDecision::Accept);
    assert_eq!(
        second.decision,
        HeartbeatValidationDecision::IgnoreDuplicate
    );
}

#[test]
fn oversized_heartbeat_is_rejected_before_binary_parse() {
    let peer = libp2p::PeerId::random();
    let cfg = MessageSecurityConfig {
        max_heartbeat_wire_bytes: 8,
        ..MessageSecurityConfig::default()
    };
    let mut cache = HeartbeatReplayCache::new(&cfg);
    let oversized = vec![b'{'; 9];

    let result = validate_heartbeat_wire(peer, &oversized, 0, &cfg, &mut cache);
    assert_eq!(result.decision, HeartbeatValidationDecision::RejectOversize);
    assert!(result.envelope.is_none());
}

#[test]
fn wrong_schema_version_is_rejected() {
    let peer = libp2p::PeerId::random();
    let cfg = MessageSecurityConfig::default();
    let mut cache = HeartbeatReplayCache::new(&cfg);
    let mut env = HeartbeatEnvelope::new(peer);
    env.schema_version = 999;
    let data = encode_heartbeat_wire(&env).unwrap();

    let result = validate_heartbeat_wire(peer, &data, env.timestamp_ns, &cfg, &mut cache);
    assert_eq!(result.decision, HeartbeatValidationDecision::Reject);
}

#[test]
fn invalid_message_security_config_fails_validation() {
    let bad_size = p2p_net::NodeConfig {
        message_security: MessageSecurityConfig {
            max_heartbeat_wire_bytes: 0,
            ..MessageSecurityConfig::default()
        },
        ..p2p_net::NodeConfig::default()
    };
    assert!(bad_size.validate().is_err());

    let bad_reputation = p2p_net::NodeConfig {
        message_security: MessageSecurityConfig {
            reputation: p2p_net::ReputationConfig {
                invalid_penalty: 0,
                ..p2p_net::ReputationConfig::default()
            },
            ..MessageSecurityConfig::default()
        },
        ..p2p_net::NodeConfig::default()
    };
    assert!(bad_reputation.validate().is_err());
}
