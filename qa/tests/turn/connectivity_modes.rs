//! Browser WebRTC and TURN are independent capabilities. These tests pin each
//! supported combination (and the rejected conflicts) through the public
//! config API, resolved as a browser node would resolve it.

use p2p_net::{
    BrowserWebRtcConfig, EnvironmentConfig, IceServer, IceTransportPolicy, NodeConfig,
    PlatformKind, ResolvedNodeConfig, StunConfig, TurnSource,
};

fn browser(json: &str) -> NodeConfig {
    let mut cfg: NodeConfig = serde_json::from_str(json).expect("config parses");
    cfg.environment = EnvironmentConfig {
        platform_hint: Some(PlatformKind::Wasm),
        ..EnvironmentConfig::default()
    };
    cfg
}

fn resolve(cfg: &NodeConfig) -> ResolvedNodeConfig {
    cfg.validate().expect("config validates");
    cfg.try_resolved_for_environment(&cfg.environment_report())
        .expect("browser config resolves")
}

fn load(path: &str) -> NodeConfig {
    let mut cfg = NodeConfig::load_json_file(path).unwrap_or_else(|err| panic!("{path}: {err}"));
    cfg.environment.platform_hint = Some(PlatformKind::Wasm);
    cfg
}

fn turn_entry(host: &str) -> IceServer {
    IceServer {
        urls: vec![
            format!("stun:{host}:3478"),
            format!("turn:{host}:3478?transport=udp"),
        ],
        username: Some("u".to_string()),
        credential: Some("c".to_string()),
    }
}

/// The ICE servers a browser would hand to a peer connection for `cfg`,
/// with `learned` standing in for TURN servers its sources produced.
fn ice_for(cfg: &NodeConfig, learned: &[IceServer]) -> Vec<IceServer> {
    let client = cfg.browser_webrtc.with_legacy_turn(&cfg.turn.client);
    let mut turn: Vec<IceServer> = client.servers.clone();
    turn.extend_from_slice(learned);
    cfg.browser_webrtc.plan().compose(&turn)
}

#[test]
fn defaults_are_the_automatic_mode() {
    let cfg = browser("{}");
    let resolved = resolve(&cfg);
    assert!(resolved.transport_capabilities.browser_webrtc);
    assert!(resolved.transport_capabilities.relay_assisted_upgrade);
    assert!(resolved.turn_network_client_enabled);
    assert_eq!(
        cfg.browser_webrtc.effective_policy(),
        IceTransportPolicy::All
    );
    assert_eq!(cfg.turn.client.sources(), vec![TurnSource::Libp2p]);
    assert!(
        ice_for(&cfg, &[]).is_empty(),
        "no third-party server is contacted by default"
    );
}

#[test]
fn webrtc_without_turn() {
    let cfg = browser(r#"{"turn": {"client": {"enabled": false}}}"#);
    let resolved = resolve(&cfg);
    assert!(resolved.transport_capabilities.browser_webrtc);
    assert!(!resolved.turn_network_client_enabled);
    assert!(cfg.turn.client.sources().is_empty());
    assert_eq!(
        cfg.browser_webrtc.effective_policy(),
        IceTransportPolicy::All
    );
}

#[test]
fn stun_only() {
    let cfg = browser(
        r#"{"browser_webrtc": {"stun": {"servers": ["stun:stun.example.org:3478"]}},
            "turn": {"client": {"enabled": false}}}"#,
    );
    resolve(&cfg);
    assert_eq!(
        ice_for(&cfg, &[]),
        vec![IceServer::stun("stun:stun.example.org:3478")]
    );
}

#[test]
fn direct_with_turn_fallback() {
    let cfg = load("examples/browser-turn-client.config.json");
    let resolved = resolve(&cfg);
    assert!(resolved.turn_network_client_enabled);
    assert_eq!(
        cfg.browser_webrtc.effective_policy(),
        IceTransportPolicy::All
    );
    let ice = ice_for(&cfg, &[turn_entry("turn.cloudflare.com")]);
    assert_eq!(ice[0], IceServer::stun("stun:stun.cloudflare.com:3478"));
    assert!(ice.iter().skip(1).all(IceServer::is_turn));
    assert_eq!(ice.len(), 3, "STUN, self-hosted TURN, Cloudflare TURN");
}

#[test]
fn turn_only_for_privacy_never_offers_direct_or_stun_candidates() {
    let cfg = load("examples/browser-turn-only-privacy.config.json");
    resolve(&cfg);
    assert_eq!(
        cfg.browser_webrtc.effective_policy(),
        IceTransportPolicy::Relay
    );
    let ice = ice_for(&cfg, &[turn_entry("turn.cloudflare.com")]);
    assert!(ice
        .iter()
        .flat_map(|server| &server.urls)
        .all(|url| url.starts_with("turn:") || url.starts_with("turns:")));

    // `direct_connections: false` alone is the same privacy policy, and a
    // TURN source can never widen it.
    let cfg = browser(r#"{"browser_webrtc": {"direct_connections": false}}"#);
    resolve(&cfg);
    assert_eq!(
        cfg.browser_webrtc.effective_policy(),
        IceTransportPolicy::Relay
    );
}

#[test]
fn webrtc_direct_only_example() {
    let cfg = load("examples/browser-webrtc-direct.config.json");
    let resolved = resolve(&cfg);
    assert!(!resolved.turn_network_client_enabled);
    assert!(ice_for(&cfg, &[]).iter().all(|server| !server.is_turn()));
}

#[test]
fn cloudflare_and_self_hosted_sources() {
    let cfg = browser(
        r#"{"turn": {"client": {
            "auto_discover": false,
            "servers": [{"urls": "turn:turn.example.org:3478", "username": "u", "credential": "c"}],
            "cloudflare": {"url": "https://p2p-net-turn.example.workers.dev/ice-servers"}}}}"#,
    );
    let resolved = resolve(&cfg);
    assert!(!resolved.turn_network_client_enabled);
    assert_eq!(
        cfg.turn.client.sources(),
        vec![TurnSource::SelfHosted, TurnSource::Cloudflare]
    );
}

#[test]
fn disabling_browser_webrtc_turns_off_signaling_and_turn_discovery() {
    let cfg = browser(r#"{"browser_webrtc": {"enabled": false}}"#);
    let resolved = resolve(&cfg);
    assert!(!resolved.transport_capabilities.browser_webrtc);
    assert!(!resolved.transport_capabilities.relay_assisted_upgrade);
    assert!(!resolved.turn_network_client_enabled);
}

#[test]
fn legacy_browser_webrtc_configs_keep_working() {
    let cfg = browser(
        r#"{"browser_webrtc": {"ice_servers": ["stun:legacy.example.org:3478"],
            "ice_transport_policy": "relay"}}"#,
    );
    resolve(&cfg);
    assert_eq!(
        cfg.browser_webrtc.effective_policy(),
        IceTransportPolicy::Relay
    );
    assert_eq!(
        cfg.browser_webrtc.stun_servers(),
        vec![IceServer::stun("stun:legacy.example.org:3478")]
    );
    assert!(
        ice_for(&cfg, &[]).is_empty(),
        "relay-only drops legacy STUN instead of leaking the address"
    );

    let legacy_turn = browser(
        r#"{"browser_webrtc": {"ice_servers": [
            {"urls": ["turn:legacy.example.org:3478"], "username": "u", "credential": "c"}]}}"#,
    );
    resolve(&legacy_turn);
    let client = legacy_turn
        .browser_webrtc
        .with_legacy_turn(&legacy_turn.turn.client);
    assert_eq!(
        client.sources(),
        vec![TurnSource::SelfHosted, TurnSource::Libp2p]
    );
}

#[test]
fn conflicting_configurations_are_rejected() {
    let conflicts = [
        // Relay-only and direct connections contradict each other.
        r#"{"browser_webrtc": {"ice_transport_policy": "relay", "direct_connections": true}}"#,
        r#"{"browser_webrtc": {"ice_transport_policy": "all", "direct_connections": false}}"#,
        // STUN switched off while STUN servers are listed.
        r#"{"browser_webrtc": {"stun": {"enabled": false, "servers": ["stun:s.example.org"]}}}"#,
        // WebRTC off, but WebRTC or TURN client settings present.
        r#"{"browser_webrtc": {"enabled": false, "ice_transport_policy": "relay"}}"#,
        r#"{"browser_webrtc": {"enabled": false},
            "turn": {"client": {"cloudflare": {"url": "https://w.example.dev/ice-servers"}}}}"#,
        // TURN client off, but TURN sources configured.
        r#"{"turn": {"client": {"enabled": false,
            "servers": [{"urls": "turn:t.example.org", "username": "u", "credential": "c"}]}}}"#,
        // Explicit providers that disagree with configuration.
        r#"{"turn": {"client": {"auto_discover": false, "providers": ["libp2p"]}}}"#,
        r#"{"turn": {"client": {"providers": ["cloudflare"]}}}"#,
        r#"{"turn": {"client": {"providers": ["libp2p"],
            "cloudflare": {"url": "https://w.example.dev/ice-servers"}}}}"#,
        // STUN entries are not TURN servers; TURN without credentials is unusable.
        r#"{"turn": {"client": {"servers": [{"urls": "stun:s.example.org"}]}}}"#,
        r#"{"browser_webrtc": {"ice_servers": ["turn:t.example.org"]}}"#,
        // Secrets in a browser config.
        r#"{"turn": {"client": {"http": {"url": "https://w.example.dev/ice",
            "bearer_token": {"env": "TOKEN"}}}}}"#,
        // Too many ICE servers per peer connection.
        r#"{"browser_webrtc": {"stun": {"servers": ["stun:a.example.org", "stun:b.example.org"]}},
            "turn": {"client": {"max_servers": 8}}}"#,
    ];
    for json in conflicts {
        let cfg = browser(json);
        assert!(
            cfg.validate().is_err(),
            "accepted conflicting config: {json}"
        );
    }
}

#[test]
fn config_round_trips_through_json() {
    let cfg = NodeConfig {
        browser_webrtc: BrowserWebRtcConfig {
            direct_connections: Some(false),
            stun: StunConfig {
                enabled: false,
                servers: Vec::new(),
            },
            ..BrowserWebRtcConfig::default()
        },
        ..NodeConfig::default()
    };
    let json = cfg.to_pretty_json().expect("encode");
    let decoded: NodeConfig = serde_json::from_str(&json).expect("decode");
    assert_eq!(decoded.browser_webrtc, cfg.browser_webrtc);
    assert_eq!(decoded.turn.client, cfg.turn.client);
}
