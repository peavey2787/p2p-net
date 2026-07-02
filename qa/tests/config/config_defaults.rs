use std::fs;

use p2p_net::{
    DnsaddrConfig, NodeConfig, NodeProfile, NodeRole, PublicFallbackMode, RelayAccess,
    RelayServiceConfig, DEFAULT_PUBLIC_BOOTSTRAP_SEED_PEERS, DEFAULT_PUBLIC_RENDEZVOUS_PEERS,
};


#[test]
fn public_fallback_is_enabled_by_default_for_normal_app_mode() {
    let cfg = NodeConfig::default();

    assert_eq!(
        cfg.discovery.public_bootstrap.mode,
        PublicFallbackMode::FallbackOnly
    );
    assert_eq!(
        cfg.discovery.public_bootstrap.bootstrap_seed_peers.len(),
        DEFAULT_PUBLIC_BOOTSTRAP_SEED_PEERS.len()
    );
    assert_eq!(
        cfg.discovery.public_bootstrap.rendezvous_peers.len(),
        DEFAULT_PUBLIC_RENDEZVOUS_PEERS.len()
    );
    assert!(cfg.discovery.public_bootstrap.relay_peers.is_empty());
    assert!(cfg.discovery.public_bootstrap.auto_connect_discovered_peers);
    assert!(cfg.discovery.rendezvous.client_enabled);
    assert!(!cfg.discovery.rendezvous.server_enabled);
    assert!(cfg.discovery.dht.discover_with_rendezvous_peers);
    assert!(cfg.discovery.public_bootstrap.bootstrap_decision(0).used);
    assert!(!cfg.discovery.public_bootstrap.bootstrap_decision(1).used);
    assert!(!cfg.discovery.public_bootstrap.rendezvous_decision(1).used);
}

#[test]
fn private_infrastructure_only_disables_public_rendezvous_reporting_policy() {
    let cfg = p2p_net::PublicBootstrapConfig::private_infrastructure_only();

    assert_eq!(cfg.mode, PublicFallbackMode::Disabled);
    assert!(!cfg.rendezvous_decision(0).used);
    assert!(!cfg.relay_decision(0).used);
    assert!(!cfg.auto_connect_discovered_peers);
}

#[test]
fn relay_disabled_by_default() {
    let cfg = NodeConfig::default();
    assert!(!cfg.relay.enabled);
    assert!(!cfg.relay.is_active_now());
    cfg.validate().expect("default config validates");
}

#[test]
fn default_profile_is_auto_and_resolves_to_current_full_role() {
    let cfg = NodeConfig::default();
    assert_eq!(cfg.profile, NodeProfile::Auto);
    let resolved = cfg.resolved();
    assert_eq!(resolved.role, NodeRole::Full);
    assert!(resolved.enabled_behaviours.gossipsub);
    assert!(resolved.enabled_behaviours.kademlia_server);
    assert!(resolved.enabled_behaviours.relay_client);
    assert!(resolved.enabled_behaviours.rendezvous_client);
    assert!(resolved.enabled_behaviours.dcutr);
    assert!(!resolved.enabled_behaviours.relay_server);
}

#[test]
fn explicit_lite_profile_disables_infrastructure_behaviours() {
    let cfg = NodeConfig {
        profile: NodeProfile::Lite,
        relay: RelayServiceConfig {
            enabled: true,
            ..RelayServiceConfig::default()
        },
        ..NodeConfig::default()
    };

    let effective = cfg.with_profile_defaults_applied();
    assert!(!effective.relay.enabled);

    let resolved = effective.resolved();
    assert_eq!(resolved.role, NodeRole::Lite);
    assert!(resolved.enabled_behaviours.relay_client);
    assert!(resolved.enabled_behaviours.dcutr);
    assert!(!resolved.enabled_behaviours.kademlia_server);
    assert!(!resolved.enabled_behaviours.relay_server);
}

#[test]
fn explicit_relay_profile_enables_mediator_capability() {
    let cfg = NodeConfig {
        profile: NodeProfile::Relay,
        relay: RelayServiceConfig {
            enabled: false,
            ..RelayServiceConfig::default()
        },
        ..NodeConfig::default()
    };

    let effective = cfg.with_profile_defaults_applied();
    assert!(effective.relay.enabled);

    let resolved = effective.resolved();
    assert_eq!(resolved.role, NodeRole::Relay);
    assert!(resolved.enabled_behaviours.relay_server);
    assert!(resolved.enabled_behaviours.kademlia_server);
}

#[test]
fn explicit_rendezvous_profile_enables_client_and_server_flags() {
    let cfg = NodeConfig {
        profile: NodeProfile::Rendezvous,
        ..NodeConfig::default()
    };

    let effective = cfg.with_profile_defaults_applied();
    assert!(effective.discovery.rendezvous.client_enabled);
    assert!(effective.discovery.rendezvous.server_enabled);

    let resolved = effective.resolved();
    assert_eq!(resolved.role, NodeRole::Rendezvous);
    assert!(resolved.enabled_behaviours.rendezvous_client);
    assert!(resolved.enabled_behaviours.rendezvous_server);
}

#[test]
fn minimal_relay_config_loads() {
    let cfg: NodeConfig = serde_json::from_str(r#"{ "relay": { "enabled": true } }"#).unwrap();
    cfg.validate().expect("minimal relay config validates");
    assert!(cfg.relay.enabled);
    assert_eq!(cfg.relay.access, RelayAccess::AllowAll);
}

#[test]
fn deny_list_wins_over_allow_list() {
    let peer = libp2p::PeerId::random();
    let relay = RelayServiceConfig {
        enabled: true,
        access: RelayAccess::AllowList,
        allow_peers: vec![peer.to_string()],
        deny_peers: vec![peer.to_string()],
        ..RelayServiceConfig::default()
    };
    assert!(!relay.allows_peer(&peer));
}

#[test]
fn invalid_peer_ids_fail_config_validation() {
    let cfg = NodeConfig {
        relay: RelayServiceConfig {
            enabled: true,
            allow_peers: vec!["not-a-peer-id".to_string()],
            ..RelayServiceConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn relay_peer_without_p2p_component_fails_config_validation() {
    let cfg = NodeConfig {
        relay_peers: vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
        ..NodeConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn generated_default_config_reloads_successfully() {
    let path = temp_path("default-config", "json");
    let text = NodeConfig::default()
        .to_pretty_json()
        .expect("render config");
    fs::write(&path, text).expect("write config");
    let loaded = NodeConfig::load_json_file(&path).expect("reload config");
    let _ = fs::remove_file(path);
    assert!(!loaded.relay.enabled);
    assert_eq!(loaded.listen_addresses.len(), 3);
}

#[test]
fn configured_listen_addresses_parse_or_fail_clearly() {
    let valid = NodeConfig {
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        ..NodeConfig::default()
    };
    assert_eq!(valid.parsed_listen_addresses().unwrap().len(), 1);

    let invalid = NodeConfig {
        listen_addresses: vec!["not-a-multiaddr".to_string()],
        ..NodeConfig::default()
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn configured_bootstrap_peer_with_invalid_peer_id_fails_validation() {
    let cfg = NodeConfig {
        bootstrap_peers: vec!["/ip4/127.0.0.1/tcp/4001/p2p/not-a-peer".to_string()],
        ..NodeConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn dns_peer_addresses_remain_supported_by_default() {
    let peer = libp2p::PeerId::random();
    let cfg = NodeConfig {
        bootstrap_peers: vec![format!("/dns4/bootstrap.example.com/tcp/4001/p2p/{peer}")],
        ..NodeConfig::default()
    };
    cfg.validate()
        .expect("ordinary DNS bootstrap address validates");
}

#[test]
fn dnsaddr_peer_addresses_validate_for_safe_resolver_path() {
    let peer = libp2p::PeerId::random();
    let cfg = NodeConfig {
        bootstrap_peers: vec![format!("/dnsaddr/bootstrap.example.com/p2p/{peer}")],
        ..NodeConfig::default()
    };
    cfg.validate().expect("dnsaddr bootstrap address validates");
}

#[test]
fn dnsaddr_doh_endpoint_is_configurable() {
    let cfg = NodeConfig {
        dnsaddr: DnsaddrConfig {
            enabled: true,
            doh_endpoint: "https://doh.internal.example/dns-query".to_string(),
            timeout_secs: 3,
        },
        ..NodeConfig::default()
    };
    cfg.validate().expect("custom DoH endpoint validates");
}

#[test]
fn dnsaddr_disabled_rejects_dnsaddr_peer_addresses() {
    let peer = libp2p::PeerId::random();
    let cfg = NodeConfig {
        dnsaddr: DnsaddrConfig {
            enabled: false,
            ..DnsaddrConfig::default()
        },
        bootstrap_peers: vec![format!("/dnsaddr/bootstrap.example.com/p2p/{peer}")],
        ..NodeConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn dnsaddr_endpoint_must_be_https_when_enabled() {
    let cfg = NodeConfig {
        dnsaddr: DnsaddrConfig {
            enabled: true,
            doh_endpoint: "http://doh.internal.example/dns-query".to_string(),
            timeout_secs: 5,
        },
        ..NodeConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn listen_addresses_reject_dns_names() {
    let cfg = NodeConfig {
        listen_addresses: vec!["/dns4/listen.example.com/tcp/4001".to_string()],
        ..NodeConfig::default()
    };
    assert!(cfg.validate().is_err());
}

fn temp_path(prefix: &str, ext: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "p2p-net-{prefix}-{}.{}",
        libp2p::PeerId::random(),
        ext
    ))
}
