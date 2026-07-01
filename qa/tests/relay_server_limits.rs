use libp2p::PeerId;
use p2p_net::connectivity::limits::{multiaddr_ip_key, ConnectionCapState};
use p2p_net::connectivity::relay::{classify_relay_denial, RelayServiceHealth};
use p2p_net::{
    ConnectionLimitsConfig, NodeConfig, RelayAccess, RelaySchedule, RelayServiceConfig, RelayWindow,
};

#[test]
fn allow_list_mode_rejects_unknown_peers() {
    let allowed = PeerId::random();
    let unknown = PeerId::random();
    let relay = RelayServiceConfig {
        enabled: true,
        access: RelayAccess::AllowList,
        allow_peers: vec![allowed.to_string()],
        ..RelayServiceConfig::default()
    };

    assert!(relay.allows_peer(&allowed));
    assert!(!relay.allows_peer(&unknown));
}

#[test]
fn relay_libp2p_config_uses_configured_limits() {
    let relay = RelayServiceConfig {
        enabled: true,
        max_reservations: 7,
        max_reservations_per_peer: 2,
        reservation_duration_secs: 11,
        max_circuits: 13,
        max_circuits_per_peer: 3,
        max_circuit_duration_secs: 17,
        max_circuit_bytes: 19,
        reservation_rate_per_peer_per_min: 5,
        reservation_rate_per_ip_per_min: 6,
        circuit_rate_per_peer_per_min: 7,
        circuit_rate_per_ip_per_min: 8,
        ..RelayServiceConfig::default()
    };

    let cfg = relay.to_libp2p_config();
    assert_eq!(cfg.max_reservations, 7);
    assert_eq!(cfg.max_reservations_per_peer, 2);
    assert_eq!(cfg.reservation_duration.as_secs(), 11);
    assert_eq!(cfg.max_circuits, 13);
    assert_eq!(cfg.max_circuits_per_peer, 3);
    assert_eq!(cfg.max_circuit_duration.as_secs(), 17);
    assert_eq!(cfg.max_circuit_bytes, 19);
    assert_eq!(cfg.reservation_rate_limiters.len(), 2);
    assert_eq!(cfg.circuit_src_rate_limiters.len(), 2);
}

#[test]
fn invalid_relay_limit_config_fails_validation() {
    let too_many_per_peer = NodeConfig {
        relay: RelayServiceConfig {
            enabled: true,
            max_reservations: 1,
            max_reservations_per_peer: 2,
            ..RelayServiceConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(too_many_per_peer.validate().is_err());

    let zero_circuit_bytes = NodeConfig {
        relay: RelayServiceConfig {
            enabled: true,
            max_circuit_bytes: 0,
            ..RelayServiceConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(zero_circuit_bytes.validate().is_err());
}

#[test]
fn relay_schedule_opens_and_closes_at_utc_times() {
    let schedule = RelaySchedule {
        enabled: true,
        windows: vec![RelayWindow {
            days: vec!["mon".to_string()],
            start: "18:00".to_string(),
            end: "23:00".to_string(),
        }],
    };

    assert!(!schedule.is_open_at_utc(1, 17 * 60 + 59));
    assert!(schedule.is_open_at_utc(1, 18 * 60));
    assert!(schedule.is_open_at_utc(1, 22 * 60 + 59));
    assert!(!schedule.is_open_at_utc(1, 23 * 60));
    assert!(!schedule.is_open_at_utc(2, 19 * 60));
}

#[test]
fn overnight_relay_schedule_matches_next_day_until_end() {
    let schedule = RelaySchedule {
        enabled: true,
        windows: vec![RelayWindow {
            days: vec!["fri".to_string()],
            start: "22:00".to_string(),
            end: "02:00".to_string(),
        }],
    };

    assert!(schedule.is_open_at_utc(5, 23 * 60));
    assert!(schedule.is_open_at_utc(6, 60));
    assert!(!schedule.is_open_at_utc(6, 2 * 60));
}

#[test]
fn relay_health_reflects_schedule_state() {
    let closed = RelayServiceConfig {
        enabled: true,
        schedule: RelaySchedule {
            enabled: true,
            windows: vec![],
        },
        ..RelayServiceConfig::default()
    };
    assert_eq!(closed.health_now(), RelayServiceHealth::ClosedBySchedule);

    let disabled = RelayServiceConfig::default();
    assert_eq!(disabled.health_now(), RelayServiceHealth::Disabled);
}

#[test]
fn connection_limit_config_validates_safe_defaults_and_rejects_zero() {
    NodeConfig::default()
        .validate()
        .expect("default config validates");

    let bad = NodeConfig {
        connection_limits: ConnectionLimitsConfig {
            max_established_per_peer: Some(0),
            ..ConnectionLimitsConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(bad.validate().is_err());
}

#[test]
fn per_ip_connection_cap_tracker_counts_and_flags_excess() {
    let cfg = ConnectionLimitsConfig {
        max_established_per_ip: Some(1),
        ..ConnectionLimitsConfig::default()
    };
    let mut caps = ConnectionCapState::new(&cfg);
    let addr: libp2p::Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();

    let first = libp2p::swarm::ConnectionId::new_unchecked(1);
    let second = libp2p::swarm::ConnectionId::new_unchecked(2);

    assert_eq!(multiaddr_ip_key(&addr).as_deref(), Some("127.0.0.1"));
    assert!(!caps.record_established(first, &addr));
    assert_eq!(caps.count_for_ip("127.0.0.1"), 1);
    assert!(caps.record_established(second, &addr));
    assert_eq!(caps.cap_disconnects, 1);
    caps.record_closed(first);
    assert_eq!(caps.count_for_ip("127.0.0.1"), 1);
}

#[test]
fn relay_denial_classification_sets_health_bucket() {
    assert_eq!(
        classify_relay_denial("RateLimited"),
        RelayServiceHealth::RateLimited
    );
    assert_eq!(
        classify_relay_denial("ResourceLimitExceeded"),
        RelayServiceHealth::AtCapacity
    );
    assert_eq!(
        classify_relay_denial("PermissionDenied"),
        RelayServiceHealth::Error
    );
}
