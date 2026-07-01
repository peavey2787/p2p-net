use p2p_net::{
    apply_resolved_capabilities, EnvironmentConfig, MediatorConfig, NodeConfig, NodeProfile,
    NodeRole, RelayAccess, RelayServiceConfig,
};

#[test]
fn mediator_profile_enables_first_class_mediator_role() {
    let cfg = NodeConfig {
        profile: NodeProfile::Mediator,
        ..NodeConfig::default()
    };

    let effective = cfg.with_profile_defaults_applied();
    assert!(effective.mediator.enabled);
    assert!(effective.relay.enabled);

    let resolved = cfg.try_resolved().expect("mediator profile resolves");
    assert_eq!(resolved.role, NodeRole::Mediator);
    assert!(resolved.mediator_enabled);
    assert!(resolved.mediator_advertise_for_dcutr);
    assert!(resolved.enabled_behaviours.relay_server);
    assert!(resolved.enabled_behaviours.dcutr);
    assert!(resolved.enabled_behaviours.kademlia_server);
}

#[test]
fn auto_profile_with_mediator_enabled_resolves_to_mediator() {
    let cfg = NodeConfig {
        profile: NodeProfile::Auto,
        mediator: MediatorConfig {
            enabled: true,
            max_reservations: 8,
            max_circuits: 16,
            ..MediatorConfig::default()
        },
        ..NodeConfig::default()
    };

    let environment = cfg.environment_report();
    let resolved = cfg
        .try_resolved_for_environment(&environment)
        .expect("mediator capabilities resolve");
    let runtime_cfg = apply_resolved_capabilities(&cfg, &resolved);

    assert_eq!(resolved.role, NodeRole::Mediator);
    assert!(runtime_cfg.relay.enabled);
    assert_eq!(runtime_cfg.relay.max_reservations, 8);
    assert_eq!(runtime_cfg.relay.max_circuits, 16);
}

#[test]
fn authenticated_mediator_requires_trusted_relay_allow_list() {
    let cfg = NodeConfig {
        mediator: MediatorConfig {
            enabled: true,
            require_authenticated_peers: true,
            ..MediatorConfig::default()
        },
        relay: RelayServiceConfig {
            access: RelayAccess::AllowList,
            ..RelayServiceConfig::default()
        },
        ..NodeConfig::default()
    };

    assert!(cfg.validate().is_err());
}

#[test]
fn background_restricted_mediator_is_rejected_before_startup() {
    let cfg = NodeConfig {
        profile: NodeProfile::Mediator,
        environment: EnvironmentConfig {
            background_restricted: Some(true),
            ..EnvironmentConfig::default()
        },
        ..NodeConfig::default()
    };

    let environment = cfg.environment_report();
    let err = cfg
        .try_resolved_for_environment(&environment)
        .expect_err("background-restricted mediator must fail");

    assert!(err.to_string().contains("background-restricted"));
}
