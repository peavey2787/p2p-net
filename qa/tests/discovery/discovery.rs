use std::fs;

use libp2p::{Multiaddr, PeerId};
use p2p_net::connectivity::peer_cache::{
    is_cacheable_peer_addr, load_identities, load_last_addrs, record_peer_addr_failure,
    record_seen_peer_addr,
};
use p2p_net::stack::{startup_discovery_plan, startup_discovery_plan_with_public};
use p2p_net::{start_node, DiscoveryConfig, NodeConfig, PublicBootstrapConfig, PublicFallbackMode};

#[test]
fn peer_cache_persists_public_p2p_addresses() {
    let peer = PeerId::random();
    let path = temp_path("peer-cache-persists");
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        ..DiscoveryConfig::default()
    };
    let bare: Multiaddr = "/ip4/203.0.113.10/tcp/4001".parse().unwrap();

    record_seen_peer_addr(&cfg, &peer, &bare);
    let cached = load_last_addrs(&cfg, 8);
    let _ = fs::remove_file(path);

    assert_eq!(cached.len(), 1);
    assert_eq!(
        cached[0].to_string(),
        format!("/ip4/203.0.113.10/tcp/4001/p2p/{peer}")
    );
}

#[test]
fn bad_cache_entries_are_ignored() {
    let peer = PeerId::random();
    let path = temp_path("bad-cache-ignored");
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        peer_cache_max_age_secs: 10,
        peer_cache_max_failures: 3,
        ..DiscoveryConfig::default()
    };
    let raw = format!(
        r#"{{
          "version": 1,
          "entries": [
            {{ "peer_id": "not-a-peer", "addr": "/ip4/127.0.0.1/tcp/1/p2p/not-a-peer", "last_seen_unix_secs": 1, "failures": 0 }},
            {{ "peer_id": "{peer}", "addr": "/ip4/127.0.0.1/tcp/2", "last_seen_unix_secs": 1, "failures": 0 }},
            {{ "peer_id": "{peer}", "addr": "/ip4/0.0.0.0/tcp/3/p2p/{peer}", "last_seen_unix_secs": 1, "failures": 0 }},
            {{ "peer_id": "{peer}", "addr": "/ip4/127.0.0.1/tcp/4/p2p/{peer}", "last_seen_unix_secs": 1, "failures": 3 }}
          ]
        }}"#
    );
    fs::write(&path, raw).unwrap();

    let cached = load_last_addrs(&cfg, 8);
    let _ = fs::remove_file(path);
    assert!(cached.is_empty());
}

#[test]
fn peer_cache_evicts_by_failure_count() {
    let peer = PeerId::random();
    let path = temp_path("failure-eviction");
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        peer_cache_max_failures: 1,
        ..DiscoveryConfig::default()
    };
    let bare: Multiaddr = "/ip4/203.0.113.10/tcp/4001".parse().unwrap();

    record_seen_peer_addr(&cfg, &peer, &bare);
    assert_eq!(load_last_addrs(&cfg, 8).len(), 1);
    record_peer_addr_failure(&cfg, &peer);
    let cached = load_last_addrs(&cfg, 8);
    let identities = load_identities(&cfg);
    let _ = fs::remove_file(path);
    assert!(cached.is_empty());
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].peer_id, peer.to_string());
}

#[test]
fn local_peer_cache_addrs_are_session_only_by_default() {
    let peer = PeerId::random();
    let path = temp_path("local-session-only");
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        ..DiscoveryConfig::default()
    };
    let bare: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();

    record_seen_peer_addr(&cfg, &peer, &bare);
    let cached = load_last_addrs(&cfg, 8);
    let identities = load_identities(&cfg);
    let _ = fs::remove_file(path);

    assert!(cached.is_empty());
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].peer_id, peer.to_string());
}

#[test]
fn startup_plan_dials_bootstrap_seed_rendezvous_and_cached_peers() {
    let bootstrap = p2p_addr(1001);
    let seed = p2p_addr(1002);
    let rendezvous = p2p_addr(1003);
    let cached = p2p_addr(1004);

    let plan = startup_discovery_plan(
        vec![bootstrap.clone()],
        vec![seed.clone()],
        vec![rendezvous.clone()],
        vec![cached.clone()],
    );

    assert_eq!(plan.bootstrap_peer_count, 1);
    assert_eq!(plan.bootstrap_seed_count, 1);
    assert_eq!(plan.rendezvous_seed_count, 1);
    assert_eq!(plan.cached_peer_count, 1);
    assert_eq!(plan.dial_addrs, vec![bootstrap, seed, rendezvous, cached]);
}

#[test]
fn startup_plan_ignores_bare_cached_addrs() {
    let bare: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
    let plan = startup_discovery_plan(Vec::new(), Vec::new(), Vec::new(), vec![bare]);
    assert!(plan.dial_addrs.is_empty());
}

#[test]
fn startup_plan_can_add_public_bootstrap_fallback_after_owned_candidates() {
    let owned = p2p_addr(1101);
    let public = p2p_addr(1102);
    let plan = startup_discovery_plan_with_public(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![owned.clone()],
        vec![public.clone()],
        true,
    );

    assert!(plan.public_fallback_used);
    assert_eq!(plan.public_bootstrap_seed_count, 1);
    assert_eq!(plan.dial_addrs, vec![owned, public]);
}

#[test]
fn public_bootstrap_policy_is_explicit_and_prefers_owned_candidates() {
    let cfg = PublicBootstrapConfig {
        mode: PublicFallbackMode::FallbackOnly,
        bootstrap_seed_peers: vec![p2p_addr(1201).to_string()],
        rendezvous_peers: vec![p2p_addr(1203).to_string()],
        relay_peers: vec![p2p_addr(1202).to_string()],
        auto_connect_discovered_peers: true,
    };

    assert!(!cfg.bootstrap_decision(1).used);
    assert!(cfg.bootstrap_decision(0).used);
    assert!(!cfg.rendezvous_decision(1).used);
    assert!(cfg.rendezvous_decision(0).used);
    assert!(!cfg.relay_decision(1).used);
    assert!(cfg.relay_decision(0).used);
}

#[test]
fn bootstrap_seed_config_requires_p2p_peer_id() {
    let bad = NodeConfig {
        discovery: DiscoveryConfig {
            bootstrap_seed_peers: vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(bad.validate().is_err());

    let good = NodeConfig {
        discovery: DiscoveryConfig {
            bootstrap_seed_peers: vec![p2p_addr(4001).to_string()],
            rendezvous_peers: vec![p2p_addr(4002).to_string()],
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    good.validate().expect("seed config validates");
}

#[test]
fn public_bootstrap_config_requires_p2p_peer_id_when_present() {
    let bad = NodeConfig {
        discovery: DiscoveryConfig {
            public_bootstrap: PublicBootstrapConfig {
                mode: PublicFallbackMode::FallbackOnly,
                bootstrap_seed_peers: vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
                rendezvous_peers: Vec::new(),
                relay_peers: Vec::new(),
                auto_connect_discovered_peers: true,
            },
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(bad.validate().is_err());

    let good = NodeConfig {
        discovery: DiscoveryConfig {
            public_bootstrap: PublicBootstrapConfig {
                mode: PublicFallbackMode::FallbackOnly,
                bootstrap_seed_peers: vec![p2p_addr(4003).to_string()],
                rendezvous_peers: vec![p2p_addr(4005).to_string()],
                relay_peers: vec![p2p_addr(4004).to_string()],
                auto_connect_discovered_peers: true,
            },
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    good.validate().expect("public fallback config validates");
}

#[test]
fn enabled_public_bootstrap_requires_at_least_one_public_candidate() {
    let cfg = DiscoveryConfig {
        public_bootstrap: PublicBootstrapConfig {
            mode: PublicFallbackMode::FallbackOnly,
            bootstrap_seed_peers: Vec::new(),
            rendezvous_peers: Vec::new(),
            relay_peers: Vec::new(),
            auto_connect_discovered_peers: true,
        },
        ..DiscoveryConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn cacheable_peer_addr_requires_identity_bound_reachable_addr() {
    let peer = PeerId::random();
    let good: Multiaddr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{peer}")
        .parse()
        .unwrap();
    let bare: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
    let unspecified: Multiaddr = format!("/ip4/0.0.0.0/tcp/4001/p2p/{peer}").parse().unwrap();

    assert!(is_cacheable_peer_addr(&good, Some(&peer)));
    assert!(!is_cacheable_peer_addr(&bare, None));
    assert!(!is_cacheable_peer_addr(&unspecified, Some(&peer)));
}

#[tokio::test]
async fn failed_bootstrap_does_not_crash_startup() {
    let seed = p2p_addr(9);
    let cfg = NodeConfig {
        identity_key_path: temp_path("failed-bootstrap-key")
            .to_string_lossy()
            .to_string(),
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        bootstrap_peers: vec![seed.to_string()],
        discovery: DiscoveryConfig {
            peer_cache_path: temp_path("failed-bootstrap-cache")
                .to_string_lossy()
                .to_string(),
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    let key_path = cfg.identity_key_path.clone();
    let cache_path = cfg.discovery.peer_cache_path.clone();
    let handle = start_node(cfg)
        .await
        .expect("startup should not require bootstrap success");
    handle.shutdown().await;
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(cache_path);
}

fn p2p_addr(port: u16) -> Multiaddr {
    let peer = PeerId::random();
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer}")
        .parse()
        .unwrap()
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("p2p-net-{prefix}-{}", PeerId::random()))
}
