use std::fs;

use libp2p::PeerId;
use p2p_net::connectivity::{limits::ConnectionCapState, peer_cache};
use p2p_net::{
    encode_heartbeat_wire, validate_heartbeat_wire, ConnectionLimitsConfig, DiscoveryConfig,
    HeartbeatEnvelope, HeartbeatReplayCache, HeartbeatValidationDecision, MessageSecurityConfig,
    NodeConfig, RelayServiceConfig,
};

#[test]
fn thousand_malformed_gossip_messages_are_rejected() {
    let peer = PeerId::random();
    let cfg = MessageSecurityConfig::default();
    let mut cache = HeartbeatReplayCache::new(&cfg);

    for i in 0..1_000usize {
        let payload = format!("{{\"schema_version\":1,\"broken\":{i}");
        let result = validate_heartbeat_wire(peer, payload.as_bytes(), 0, &cfg, &mut cache);
        assert_eq!(result.decision, HeartbeatValidationDecision::Reject);
        assert!(result.envelope.is_none());
    }
}

#[test]
fn oversized_heartbeat_is_rejected_before_binary_parse() {
    let peer = PeerId::random();
    let cfg = MessageSecurityConfig {
        max_heartbeat_wire_bytes: 64,
        ..MessageSecurityConfig::default()
    };
    let mut cache = HeartbeatReplayCache::new(&cfg);
    let payload = vec![b'{'; cfg.max_heartbeat_wire_bytes + 1];

    let result = validate_heartbeat_wire(peer, &payload, 0, &cfg, &mut cache);
    assert_eq!(result.decision, HeartbeatValidationDecision::RejectOversize);
    assert!(result.envelope.is_none());
}

#[test]
fn replay_old_heartbeat_is_ignored_without_poisoning_cache() {
    let peer = PeerId::random();
    let cfg = MessageSecurityConfig::default();
    let mut cache = HeartbeatReplayCache::new(&cfg);
    let env = HeartbeatEnvelope::new(peer);
    let data = encode_heartbeat_wire(&env).expect("heartbeat wire");

    let first = validate_heartbeat_wire(peer, &data, env.timestamp_ns, &cfg, &mut cache);
    let replay = validate_heartbeat_wire(peer, &data, env.timestamp_ns, &cfg, &mut cache);

    assert_eq!(first.decision, HeartbeatValidationDecision::Accept);
    assert_eq!(
        replay.decision,
        HeartbeatValidationDecision::IgnoreDuplicate
    );
}

#[test]
fn spoofed_heartbeat_is_rejected_under_hostile_load() {
    let real_source = PeerId::random();
    let cfg = MessageSecurityConfig::default();
    let mut cache = HeartbeatReplayCache::new(&cfg);

    for _ in 0..128 {
        let claimed_peer = PeerId::random();
        let env = HeartbeatEnvelope::new(claimed_peer);
        let data = encode_heartbeat_wire(&env).expect("heartbeat wire");
        let result =
            validate_heartbeat_wire(real_source, &data, env.timestamp_ns, &cfg, &mut cache);
        assert_eq!(result.decision, HeartbeatValidationDecision::Reject);
    }
}

#[test]
fn many_connections_from_same_ip_hit_the_ip_cap() {
    let cfg = ConnectionLimitsConfig {
        max_established_per_ip: Some(4),
        ..ConnectionLimitsConfig::default()
    };
    let mut caps = ConnectionCapState::new(&cfg);
    let addr: libp2p::Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();

    for i in 0..4 {
        assert!(!caps.record_established(
            libp2p::swarm::ConnectionId::new_unchecked(i),
            PeerId::random(),
            &addr,
            false,
        ));
    }
    assert!(caps.record_established(
        libp2p::swarm::ConnectionId::new_unchecked(99),
        PeerId::random(),
        &addr,
        false,
    ));
    assert_eq!(caps.cap_disconnects, 1);
}

#[test]
fn same_peer_connection_cap_is_configured_and_validated() {
    let cfg = NodeConfig {
        connection_limits: ConnectionLimitsConfig {
            max_established: Some(64),
            max_established_per_peer: Some(2),
            ..ConnectionLimitsConfig::default()
        },
        ..NodeConfig::default()
    };
    cfg.validate().expect("same-peer cap config validates");

    let bad = NodeConfig {
        connection_limits: ConnectionLimitsConfig {
            max_established: Some(1),
            max_established_per_peer: Some(2),
            max_established_per_ip: Some(1),
            ..ConnectionLimitsConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(bad.validate().is_err());
}

#[test]
fn invalid_relay_config_is_rejected_not_silently_clamped() {
    let bad = NodeConfig {
        relay: RelayServiceConfig {
            enabled: true,
            max_reservations: 1,
            max_reservations_per_peer: 2,
            ..RelayServiceConfig::default()
        },
        ..NodeConfig::default()
    };

    assert!(bad.validate().is_err());
}

#[test]
fn corrupt_peer_cache_is_ignored_without_panic() {
    let path = temp_path("corrupt-cache");
    fs::write(&path, b"{ definitely not valid json").expect("write corrupt cache");
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        ..DiscoveryConfig::default()
    };

    let entries = peer_cache::load_entries(&cfg);
    let addrs = peer_cache::load_last_addrs(&cfg, 16);
    let _ = fs::remove_file(&path);

    assert!(entries.is_empty());
    assert!(addrs.is_empty());
}

#[test]
fn hostile_peer_cache_entries_are_ignored() {
    let peer = PeerId::random();
    let other = PeerId::random();
    let path = temp_path("hostile-cache");
    let unspecified_addr = format!("/ip4/0.0.0.0/tcp/4001/p2p/{peer}");
    let mismatched_addr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{other}");
    let payload = serde_json::json!({
        "version": 1,
        "entries": [
            { "peer_id": "not-a-peer", "addr": "/ip4/127.0.0.1/tcp/4001/p2p/not-a-peer", "last_seen_unix_secs": 1, "failures": 0 },
            { "peer_id": peer.to_string(), "addr": unspecified_addr, "last_seen_unix_secs": 1, "failures": 0 },
            { "peer_id": peer.to_string(), "addr": mismatched_addr, "last_seen_unix_secs": 1, "failures": 0 }
        ],
        "addrs": [
            "/ip4/127.0.0.1/tcp/4001",
            format!("/ip4/0.0.0.0/tcp/4001/p2p/{peer}")
        ]
    });
    fs::write(&path, serde_json::to_vec_pretty(&payload).expect("json")).expect("write cache");
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        ..DiscoveryConfig::default()
    };

    let entries = peer_cache::load_entries(&cfg);
    let _ = fs::remove_file(&path);

    assert!(entries.is_empty());
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("p2p-net-hostile-{prefix}-{}", PeerId::random()))
}
