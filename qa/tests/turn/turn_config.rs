//! TURN configuration, role resolution and example configs.

use p2p_net::{
    CloudflareProviderConfig, DiscoveryTrust, EnvironmentConfig, IceServer, NodeConfig,
    NodeProfile, NodeRole, SecretRef, TurnProviderConfig, TurnSource,
};

fn cloudflare() -> TurnProviderConfig {
    TurnProviderConfig::Cloudflare(CloudflareProviderConfig::new(
        "abc123",
        SecretRef::Env("CLOUDFLARE_TURN_API_TOKEN".to_string()),
    ))
}

fn load(path: &str) -> NodeConfig {
    NodeConfig::load_json_file(path).unwrap_or_else(|err| panic!("{path}: {err}"))
}

#[test]
fn defaults_contact_no_turn_service_and_run_no_server() {
    let cfg = NodeConfig::default();
    assert!(cfg.turn.client.servers.is_empty());
    assert!(cfg.turn.client.cloudflare.is_none() && cfg.turn.client.http.is_none());
    assert_eq!(cfg.turn.client.sources(), vec![TurnSource::Libp2p]);
    assert_eq!(cfg.turn.client.discovery_trust, DiscoveryTrust::SameHost);
    assert!(!cfg.turn.server.enabled);
    assert!(!cfg.turn.broker.enabled);
    let resolved = cfg.try_resolved().expect("default resolves");
    assert!(!resolved.turn_server_enabled);
    assert!(!resolved.turn_broker_enabled);
    assert!(
        !resolved.turn_network_client_enabled,
        "native nodes never ask for ICE servers"
    );
}

#[test]
fn turn_profile_is_a_one_word_turn_server_and_broker() {
    let cfg = NodeConfig {
        profile: NodeProfile::Turn,
        ..NodeConfig::default()
    };
    cfg.validate().expect("turn profile validates");
    let resolved = cfg.try_resolved().expect("turn profile resolves");
    assert_eq!(resolved.role, NodeRole::Turn);
    assert!(resolved.turn_server_enabled);
    assert!(resolved.turn_broker_enabled);
    assert!(resolved.enabled_behaviours.kademlia_server);
    let effective = cfg.with_resolved_capabilities_applied(&resolved);
    assert!(effective.turn.server.enabled && effective.turn.broker.enabled);
}

#[test]
fn enabling_the_server_on_auto_resolves_to_the_turn_role() {
    let mut cfg = NodeConfig::default();
    cfg.turn.server.enabled = true;
    assert_eq!(cfg.try_resolved().expect("resolves").role, NodeRole::Turn);
}

#[test]
fn lite_mobile_and_background_restricted_nodes_cannot_run_turn() {
    for profile in [NodeProfile::Lite, NodeProfile::MobileLite] {
        let mut cfg = NodeConfig {
            profile,
            ..NodeConfig::default()
        };
        cfg.turn.server.enabled = true;
        assert!(
            cfg.try_resolved().is_err(),
            "{profile:?} must reject turn.server"
        );
    }
    let cfg = NodeConfig {
        profile: NodeProfile::Turn,
        environment: EnvironmentConfig {
            background_restricted: Some(true),
            ..EnvironmentConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(cfg.try_resolved().is_err());
}

#[test]
fn broker_needs_a_credential_source() {
    let mut cfg = NodeConfig::default();
    cfg.turn.broker.enabled = true;
    assert!(cfg.validate().is_err());
    cfg.turn.broker.provider = Some(cloudflare());
    cfg.validate().expect("cloudflare-backed broker");
}

#[test]
fn server_needs_a_relay_address_source() {
    let mut cfg = NodeConfig {
        profile: NodeProfile::Turn,
        ..NodeConfig::default()
    };
    cfg.public_ip_probe.enabled = false;
    assert!(cfg.validate().is_err());
    cfg.turn.server.public_ip = Some("203.0.113.10".to_string());
    cfg.validate().expect("explicit relay address");
}

#[test]
fn server_rejects_overlapping_or_privileged_ports() {
    let mut cfg = NodeConfig {
        profile: NodeProfile::Turn,
        ..NodeConfig::default()
    };
    cfg.turn.server.relay_port_min = 80;
    assert!(cfg.validate().is_err());
    cfg.turn.server.relay_port_min = 3000;
    cfg.turn.server.relay_port_max = 4000;
    assert!(
        cfg.validate().is_err(),
        "listen port 3478 inside relay range"
    );
}

#[test]
fn static_turn_entries_need_credentials() {
    let mut cfg = NodeConfig::default();
    cfg.turn.client.servers = vec![IceServer::stun("turn:turn.example.org:3478")];
    assert!(cfg.validate().is_err());
    cfg.turn.client.servers = vec![IceServer::stun("stun:stun.example.org:3478")];
    assert!(
        cfg.validate().is_err(),
        "STUN belongs to browser_webrtc.stun, not turn.client"
    );
}

#[test]
fn turn_example_configs_load_and_resolve() {
    let simple = load("examples/turn-node.config.json");
    assert_eq!(simple.try_resolved().expect("simple").role, NodeRole::Turn);

    let advanced = load("examples/turn-node-advanced.config.json");
    let resolved = advanced.try_resolved().expect("advanced");
    assert!(resolved.turn_server_enabled && resolved.relay_server_enabled);
    assert!(matches!(
        advanced.turn.broker.provider,
        Some(TurnProviderConfig::Cloudflare(_))
    ));

    let broker = load("examples/turn-cloudflare-broker.config.json");
    let resolved = broker.try_resolved().expect("broker");
    assert!(resolved.turn_broker_enabled && !resolved.turn_server_enabled);

    let browser = load("examples/browser-turn-client.config.json");
    assert_eq!(
        browser.turn.client.sources(),
        vec![
            TurnSource::Libp2p,
            TurnSource::SelfHosted,
            TurnSource::Cloudflare
        ]
    );
}

#[test]
fn profile_names_round_trip_for_one_word_role_selection() {
    for profile in NodeProfile::ALL {
        assert_eq!(profile.as_str().parse::<NodeProfile>(), Ok(profile));
        let json = serde_json::to_string(&profile).expect("encode");
        assert_eq!(json, format!("\"{}\"", profile.as_str()));
    }
    let err = "helper".parse::<NodeProfile>().expect_err("unknown role");
    assert!(err.contains("turn"));
}
