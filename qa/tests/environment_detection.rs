use p2p_net::{
    EnvironmentConfig, NatKind, NetworkReachability, NodeConfig, NodeProfile, NodeRole,
    PlatformKind,
};

#[test]
fn public_desktop_auto_resolves_to_full() {
    let cfg = config_with_environment(EnvironmentConfig {
        platform_hint: Some(PlatformKind::Linux),
        reachability_hint: Some(NetworkReachability::Public),
        nat_hint: Some(NatKind::Public),
        can_accept_inbound: Some(true),
        ..EnvironmentConfig::default()
    });

    let environment = cfg.environment_report();
    assert_eq!(environment.platform, PlatformKind::Linux);
    assert_eq!(environment.reachability, NetworkReachability::Public);
    assert!(environment.can_accept_inbound);
    assert_eq!(cfg.resolved_for_environment(&environment).role, NodeRole::Full);
}

#[test]
fn private_nat_desktop_auto_resolves_to_lite() {
    let cfg = config_with_environment(EnvironmentConfig {
        platform_hint: Some(PlatformKind::Windows),
        reachability_hint: Some(NetworkReachability::PrivateNat),
        nat_hint: Some(NatKind::Private),
        can_accept_inbound: Some(false),
        ..EnvironmentConfig::default()
    });

    let environment = cfg.environment_report();
    assert_eq!(environment.platform, PlatformKind::Windows);
    assert_eq!(environment.reachability, NetworkReachability::PrivateNat);
    assert!(!environment.can_accept_inbound);
    assert_eq!(cfg.resolved_for_environment(&environment).role, NodeRole::Lite);
}

#[test]
fn cgnat_like_desktop_auto_resolves_to_lite() {
    let cfg = config_with_environment(EnvironmentConfig {
        platform_hint: Some(PlatformKind::Linux),
        nat_hint: Some(NatKind::CgnatLikely),
        ..EnvironmentConfig::default()
    });

    let environment = cfg.environment_report();
    assert_eq!(environment.reachability, NetworkReachability::CgnatLikely);
    assert!(environment.likely_cgnat);
    assert_eq!(cfg.resolved_for_environment(&environment).role, NodeRole::Lite);
}

#[test]
fn android_auto_resolves_to_mobile_lite() {
    let cfg = config_with_environment(EnvironmentConfig {
        platform_hint: Some(PlatformKind::Android),
        ..EnvironmentConfig::default()
    });

    let environment = cfg.environment_report();
    assert!(environment.battery_sensitive);
    assert!(environment.background_restricted);
    assert_eq!(
        cfg.resolved_for_environment(&environment).role,
        NodeRole::MobileLite
    );
}

#[test]
fn ios_auto_resolves_to_mobile_lite() {
    let cfg = config_with_environment(EnvironmentConfig {
        platform_hint: Some(PlatformKind::Ios),
        ..EnvironmentConfig::default()
    });

    let environment = cfg.environment_report();
    assert!(environment.battery_sensitive);
    assert!(environment.background_restricted);
    assert_eq!(
        cfg.resolved_for_environment(&environment).role,
        NodeRole::MobileLite
    );
}

#[test]
fn unknown_platform_preserves_full_compatible_default_when_reachability_unknown() {
    let cfg = config_with_environment(EnvironmentConfig {
        platform_hint: Some(PlatformKind::Unknown),
        ..EnvironmentConfig::default()
    });

    let environment = cfg.environment_report();
    assert_eq!(environment.platform, PlatformKind::Unknown);
    assert_eq!(environment.reachability, NetworkReachability::Unknown);
    assert_eq!(cfg.resolved_for_environment(&environment).role, NodeRole::Full);
}

#[test]
fn explicit_profile_overrides_environment_detection_when_capabilities_are_valid() {
    let mut cfg = config_with_environment(EnvironmentConfig {
        platform_hint: Some(PlatformKind::Linux),
        reachability_hint: Some(NetworkReachability::PrivateNat),
        can_listen_tcp: Some(true),
        background_restricted: Some(false),
        ..EnvironmentConfig::default()
    });
    cfg.profile = NodeProfile::Relay;

    let environment = cfg.environment_report();
    assert_eq!(cfg.resolved_for_environment(&environment).role, NodeRole::Relay);
}

#[test]
fn nat_observations_update_report_without_libp2p_types() {
    let cfg = config_with_environment(EnvironmentConfig {
        platform_hint: Some(PlatformKind::Linux),
        ..EnvironmentConfig::default()
    });
    let mut environment = cfg.environment_report();

    environment.apply_nat_observation(NatKind::Public);
    assert_eq!(environment.reachability, NetworkReachability::Public);
    assert!(environment.can_accept_inbound);

    environment.apply_nat_observation(NatKind::Symmetric);
    assert_eq!(environment.reachability, NetworkReachability::CgnatLikely);
    assert!(environment.likely_cgnat);
    assert!(!environment.can_accept_inbound);
}

#[test]
fn observed_addresses_are_deduplicated() {
    let cfg = config_with_environment(EnvironmentConfig::default());
    let mut environment = cfg.environment_report();

    environment.add_observed_external_address("/ip4/203.0.113.10/tcp/4001");
    environment.add_observed_external_address("/ip4/203.0.113.10/tcp/4001");

    assert_eq!(environment.observed_external_addresses.len(), 1);
}

fn config_with_environment(environment: EnvironmentConfig) -> NodeConfig {
    NodeConfig {
        profile: NodeProfile::Auto,
        environment,
        ..NodeConfig::default()
    }
}
