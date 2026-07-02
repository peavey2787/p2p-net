use p2p_net::connectivity::relay::RelayState;
use p2p_net::{DcutrPolicy, NodeConfig, NodeProfile, NodeSnapshot};

#[test]
fn default_dcutr_policy_is_production_safe() {
    let policy = DcutrPolicy::default();

    assert!(policy.enabled);
    assert!(policy.attempt_after_relay_connection);
    assert!(policy.keep_relay_fallback);
    assert_eq!(policy.retry_interval_secs, 60);
    assert_eq!(policy.max_attempts_per_peer, 3);
    assert!(policy.validate().is_ok());
}

#[test]
fn dcutr_disabled_removes_dcutr_capability() {
    let cfg = NodeConfig {
        profile: NodeProfile::Lite,
        dcutr: DcutrPolicy {
            enabled: false,
            ..DcutrPolicy::default()
        },
        ..NodeConfig::default()
    };

    let resolved = cfg.try_resolved().expect("lite config resolves");

    assert!(!resolved.dcutr_enabled);
    assert!(!resolved.enabled_behaviours.dcutr);
    assert!(resolved.enabled_behaviours.relay_client);
}

#[test]
fn dcutr_rejects_upgrade_without_relay_fallback() {
    let cfg = NodeConfig {
        dcutr: DcutrPolicy {
            keep_relay_fallback: false,
            ..DcutrPolicy::default()
        },
        ..NodeConfig::default()
    };

    assert!(cfg.validate().is_err());
}

#[test]
fn resolved_config_exposes_dcutr_retry_policy() {
    let cfg = NodeConfig {
        profile: NodeProfile::Lite,
        dcutr: DcutrPolicy {
            retry_interval_secs: 120,
            max_attempts_per_peer: 5,
            ..DcutrPolicy::default()
        },
        ..NodeConfig::default()
    };

    let resolved = cfg.try_resolved().expect("lite config resolves");

    assert!(resolved.dcutr_enabled);
    assert!(resolved.dcutr_attempt_after_relay_connection);
    assert!(resolved.dcutr_keep_relay_fallback);
    assert_eq!(resolved.dcutr_retry_interval_secs, 120);
    assert_eq!(resolved.dcutr_max_attempts_per_peer, 5);
}

#[test]
fn relay_state_updates_dcutr_fallback_counters() {
    let state = RelayState {
        dcutr_enabled: true,
        dcutr_attempts: 4,
        dcutr_successes: 2,
        dcutr_failures: 1,
        dcutr_relay_fallbacks: 3,
        dcutr_upgrade_eligible_connections: 5,
        dcutr_retry_suppressed: 1,
        ..RelayState::default()
    };
    let mut snapshot = NodeSnapshot::default();

    snapshot.apply_relay_state(&state);

    assert!(snapshot.dcutr_enabled);
    assert_eq!(snapshot.dcutr_attempts, 4);
    assert_eq!(snapshot.dcutr_successes, 2);
    assert_eq!(snapshot.dcutr_failures, 1);
    assert_eq!(snapshot.dcutr_relay_fallbacks, 3);
    assert_eq!(snapshot.dcutr_upgrade_eligible_connections, 5);
    assert_eq!(snapshot.dcutr_retry_suppressed, 1);
}
