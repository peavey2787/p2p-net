use p2p_net::{
    apply_resolved_capabilities, resolve_node_config, EnvironmentConfig, NatKind,
    NetworkReachability, NodeConfig, NodeProfile, NodeRole, PlatformKind,
};

#[test]
fn private_nat_auto_resolves_lite_relay_reservation_policy() {
    let relay_peer = libp2p::PeerId::random();
    let cfg = NodeConfig {
        profile: NodeProfile::Auto,
        environment: EnvironmentConfig {
            platform_hint: Some(PlatformKind::Linux),
            reachability_hint: Some(NetworkReachability::PrivateNat),
            nat_hint: Some(NatKind::Private),
            can_accept_inbound: Some(false),
            ..EnvironmentConfig::default()
        },
        relay_peers: vec![format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay_peer}")],
        ..NodeConfig::default()
    };

    let environment = cfg.environment_report();
    let resolved = resolve_node_config(&cfg, &environment).expect("capabilities resolve");

    assert_eq!(resolved.role, NodeRole::Lite);
    assert!(resolved.enabled_behaviours.relay_client);
    assert!(resolved.enabled_behaviours.dcutr);
    assert!(resolved.enabled_behaviours.rendezvous_client);
    assert!(!resolved.enabled_behaviours.relay_server);
    assert!(!resolved.enabled_behaviours.kademlia_server);
    assert!(resolved.should_reserve_configured_relays);
    assert!(resolved.should_reserve_selected_relays);
    assert!(!resolved.should_seed_relay_peers);
    assert!(!resolved.should_seed_selected_relays);

    let runtime_cfg = apply_resolved_capabilities(&cfg, &resolved);
    assert!(!runtime_cfg.relay.enabled);
    assert!(runtime_cfg.discovery.rendezvous.client_enabled);
    assert!(!runtime_cfg.discovery.rendezvous.server_enabled);
}

#[test]
fn disabled_relay_reservations_seed_relay_peers_instead() {
    let relay_peer = libp2p::PeerId::random();
    let cfg = NodeConfig {
        profile: NodeProfile::Lite,
        reserve_configured_relays: false,
        relay_peers: vec![format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay_peer}")],
        ..NodeConfig::default()
    };

    let resolved = cfg.try_resolved().expect("capabilities resolve");

    assert_eq!(resolved.role, NodeRole::Lite);
    assert!(!resolved.should_reserve_configured_relays);
    assert!(!resolved.should_reserve_selected_relays);
    assert!(resolved.should_seed_relay_peers);
    assert!(resolved.should_seed_selected_relays);
}

#[test]
fn mobile_lite_runtime_config_clears_public_listen_addresses() {
    let cfg = NodeConfig {
        profile: NodeProfile::Auto,
        environment: EnvironmentConfig {
            platform_hint: Some(PlatformKind::Android),
            ..EnvironmentConfig::default()
        },
        ..NodeConfig::default()
    };

    let environment = cfg.environment_report();
    let resolved = cfg
        .try_resolved_for_environment(&environment)
        .expect("capabilities resolve");
    let runtime_cfg = cfg.with_resolved_capabilities_applied(&resolved);

    assert_eq!(resolved.role, NodeRole::MobileLite);
    assert!(!resolved.should_listen);
    assert!(runtime_cfg.listen_addresses.is_empty());
    assert!(!runtime_cfg.relay.enabled);
    assert!(runtime_cfg.discovery.rendezvous.client_enabled);
}

#[test]
fn background_restricted_relay_server_is_rejected() {
    let cfg = NodeConfig {
        profile: NodeProfile::Relay,
        environment: EnvironmentConfig {
            platform_hint: Some(PlatformKind::Android),
            can_listen_tcp: Some(true),
            can_listen_quic: Some(true),
            background_restricted: Some(true),
            ..EnvironmentConfig::default()
        },
        ..NodeConfig::default()
    };

    let environment = cfg.environment_report();
    let err = cfg
        .try_resolved_for_environment(&environment)
        .expect_err("background-restricted relay must fail");

    assert!(err.to_string().contains("background-restricted"));
}

#[test]
fn explicit_relay_profile_overrides_private_reachability_when_listening_is_supported() {
    let cfg = NodeConfig {
        profile: NodeProfile::Relay,
        environment: EnvironmentConfig {
            platform_hint: Some(PlatformKind::Linux),
            reachability_hint: Some(NetworkReachability::PrivateNat),
            can_listen_tcp: Some(true),
            can_accept_inbound: Some(false),
            background_restricted: Some(false),
            ..EnvironmentConfig::default()
        },
        ..NodeConfig::default()
    };

    let environment = cfg.environment_report();
    let resolved = cfg
        .try_resolved_for_environment(&environment)
        .expect("explicit relay profile resolves");

    assert_eq!(resolved.role, NodeRole::Relay);
    assert!(resolved.enabled_behaviours.relay_server);
}
