use libp2p::{Multiaddr, PeerId};
use p2p_net::{
    select_startup_relays, NodeConfig, NodeProfile, RelayCandidateSource, RelayDiscoveryPolicy,
};

#[test]
fn relay_discovery_selects_configured_cached_and_rendezvous_candidates() {
    let configured = p2p_addr(4101);
    let cached = p2p_addr(4102);
    let rendezvous = p2p_addr(4103);
    let duplicate_configured = configured.clone();
    let bare: Multiaddr = "/ip4/127.0.0.1/tcp/4104".parse().unwrap();
    let circuit: Multiaddr = format!("{}/p2p-circuit", p2p_addr(4105)).parse().unwrap();

    let policy = RelayDiscoveryPolicy {
        min_reservations: 2,
        max_reservations: 3,
        ..RelayDiscoveryPolicy::default()
    };

    let plan = select_startup_relays(
        &policy,
        vec![configured.clone(), duplicate_configured],
        vec![cached.clone(), bare],
        vec![rendezvous.clone(), circuit],
        Vec::new(),
    );

    assert!(plan.enabled);
    assert_eq!(plan.selected_addrs, vec![configured, cached, rendezvous]);
    assert_eq!(plan.configured_candidates, 1);
    assert_eq!(plan.cached_candidates, 1);
    assert_eq!(plan.rendezvous_candidates, 1);
    assert_eq!(plan.public_candidates, 0);
    assert_eq!(plan.ignored_candidates, 3);
    assert!(plan.errors.is_empty());
}

#[test]
fn relay_discovery_validation_rejects_impossible_limits() {
    let bad = RelayDiscoveryPolicy {
        min_reservations: 4,
        max_reservations: 3,
        ..RelayDiscoveryPolicy::default()
    };
    assert!(bad.validate().is_err());

    let no_sources = RelayDiscoveryPolicy {
        use_configured_relays: false,
        use_cached_relays: false,
        use_rendezvous_relays: false,
        ..RelayDiscoveryPolicy::default()
    };
    assert!(no_sources.validate().is_err());
}

#[test]
fn lite_profile_resolves_with_relay_discovery_policy() {
    let cfg = NodeConfig {
        profile: NodeProfile::Lite,
        relay_peers: vec![p2p_addr(4201).to_string()],
        ..NodeConfig::default()
    };

    let resolved = cfg.try_resolved().expect("lite profile resolves");

    assert!(resolved.relay_discovery_enabled);
    assert_eq!(resolved.relay_discovery_min_reservations, 1);
    assert_eq!(resolved.relay_discovery_max_reservations, 3);
    assert!(resolved.enabled_behaviours.relay_client);
    assert!(resolved.enabled_behaviours.dcutr);
}

#[test]
fn disabled_policy_falls_back_to_configured_relays_only() {
    let configured = p2p_addr(4301);
    let cached = p2p_addr(4302);
    let policy = RelayDiscoveryPolicy {
        enabled: false,
        ..RelayDiscoveryPolicy::default()
    };

    let plan = select_startup_relays(&policy, vec![configured.clone()], vec![cached], Vec::new(), Vec::new());

    assert!(!plan.enabled);
    assert_eq!(plan.selected_addrs, vec![configured]);
    assert_eq!(plan.configured_candidates, 1);
    assert_eq!(plan.cached_candidates, 0);
}

#[test]
fn public_relay_fallback_candidates_are_last_and_tracked() {
    let configured = p2p_addr(4401);
    let public = p2p_addr(4402);
    let policy = RelayDiscoveryPolicy {
        max_reservations: 2,
        ..RelayDiscoveryPolicy::default()
    };

    let plan = select_startup_relays(
        &policy,
        vec![configured.clone()],
        Vec::new(),
        Vec::new(),
        vec![public.clone()],
    );

    assert_eq!(plan.selected_addrs, vec![configured, public]);
    assert_eq!(plan.configured_candidates, 1);
    assert_eq!(plan.public_candidates, 1);
    assert_eq!(RelayCandidateSource::PublicFallback.as_str(), "public_fallback");
}

fn p2p_addr(port: u16) -> Multiaddr {
    let peer = PeerId::random();
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer}")
        .parse()
        .unwrap()
}

#[test]
fn full_profile_does_not_auto_use_cached_or_rendezvous_relays_without_operator_pin() {
    let cfg = NodeConfig {
        profile: NodeProfile::Full,
        ..NodeConfig::default()
    };

    let resolved = cfg.try_resolved().expect("full profile resolves");

    assert!(!resolved.relay_discovery_enabled);
}

#[test]
fn public_relay_fallback_selects_public_candidates_when_owned_sources_are_empty() {
    let public = p2p_addr(4501);
    let policy = RelayDiscoveryPolicy {
        min_reservations: 1,
        max_reservations: 2,
        ..RelayDiscoveryPolicy::default()
    };

    let plan = select_startup_relays(
        &policy,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![public.clone()],
    );

    assert!(plan.enabled);
    assert_eq!(plan.selected_addrs, vec![public]);
    assert_eq!(plan.configured_candidates, 0);
    assert_eq!(plan.cached_candidates, 0);
    assert_eq!(plan.rendezvous_candidates, 0);
    assert_eq!(plan.public_candidates, 1);
    assert!(plan.errors.is_empty());
}

#[test]
fn public_relay_candidates_enable_selected_relay_reservation_policy() {
    let public_relay = p2p_addr(4502).to_string();
    let cfg = NodeConfig {
        discovery: p2p_net::DiscoveryConfig {
            public_bootstrap: p2p_net::PublicBootstrapConfig {
                relay_peers: vec![public_relay],
                ..p2p_net::PublicBootstrapConfig::default()
            },
            ..p2p_net::DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };

    let resolved = cfg.try_resolved().expect("public relay config resolves");

    assert!(resolved.enabled_behaviours.relay_client);
    assert!(resolved.enabled_behaviours.dcutr);
    assert!(resolved.relay_discovery_enabled);
    assert!(!resolved.should_reserve_configured_relays);
    assert!(resolved.should_reserve_selected_relays);
    assert!(!resolved.should_seed_selected_relays);
    assert!(cfg.discovery.public_bootstrap.relay_decision(0).used);
}

#[test]
fn disabled_selected_relay_reservations_seed_public_relays_instead() {
    let public_relay = p2p_addr(4503).to_string();
    let cfg = NodeConfig {
        reserve_configured_relays: false,
        discovery: p2p_net::DiscoveryConfig {
            public_bootstrap: p2p_net::PublicBootstrapConfig {
                relay_peers: vec![public_relay],
                ..p2p_net::PublicBootstrapConfig::default()
            },
            ..p2p_net::DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };

    let resolved = cfg.try_resolved().expect("public relay config resolves");

    assert!(resolved.relay_discovery_enabled);
    assert!(!resolved.should_reserve_selected_relays);
    assert!(resolved.should_seed_selected_relays);
}
