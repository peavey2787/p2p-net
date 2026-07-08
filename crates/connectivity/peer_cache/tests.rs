use std::fs;

use libp2p::{Multiaddr, PeerId};

use super::model::PeerCacheFile;
use super::*;
use crate::connectivity::discovery::DiscoveryConfig;

fn load_raw_entries_for_tests(path: impl AsRef<std::path::Path>) -> Vec<CachedPeerAddr> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<PeerCacheFile>(&raw)
        .map(|file| file.dialable_addrs)
        .unwrap_or_default()
}

#[test]
fn normalizes_bare_addr_by_appending_peer_id() {
    let peer = PeerId::random();
    let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
    let normalized = normalize_peer_addr(&peer, &addr).expect("normalized");
    assert_eq!(
        normalized.to_string(),
        format!("/ip4/127.0.0.1/tcp/4001/p2p/{peer}")
    );
}

#[test]
fn rejects_bare_and_unspecified_cache_addrs() {
    let peer = PeerId::random();
    let bare: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
    let unspecified: Multiaddr = format!("/ip4/0.0.0.0/tcp/4001/p2p/{peer}").parse().unwrap();
    assert!(!is_cacheable_peer_addr(&bare, None));
    assert!(!is_cacheable_peer_addr(&unspecified, Some(&peer)));
}

#[test]
fn records_public_identity_bound_addr() {
    let peer = PeerId::random();
    let path = std::env::temp_dir().join(format!("p2p-cache-{peer}.json"));
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        ..DiscoveryConfig::default()
    };
    let addr: Multiaddr = "/ip4/203.0.113.10/tcp/4001".parse().unwrap();
    record_seen_peer_addr(&cfg, &peer, &addr);
    let entries = load_raw_entries_for_tests(&cfg.peer_cache_path);
    let identities = load_identities(&cfg);
    let _ = fs::remove_file(&cfg.peer_cache_path);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].peer_id, peer.to_string());
    assert_eq!(entries[0].addr_kind, CachedDialAddrKind::PublicDirect);
    assert!(entries[0].addr.ends_with(&format!("/p2p/{peer}")));
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].peer_id, peer.to_string());
}

#[test]
fn local_addrs_are_identity_only_by_default() {
    let peer = PeerId::random();
    let path = std::env::temp_dir().join(format!("p2p-cache-local-{peer}.json"));
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        ..DiscoveryConfig::default()
    };
    let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
    record_seen_peer_addr(&cfg, &peer, &addr);
    let entries = load_entries(&cfg);
    let identities = load_identities(&cfg);
    let _ = fs::remove_file(&cfg.peer_cache_path);
    assert!(entries.is_empty());
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].peer_id, peer.to_string());
}

#[test]
fn local_addrs_can_be_persisted_when_explicitly_configured() {
    let peer = PeerId::random();
    let path = std::env::temp_dir().join(format!("p2p-cache-local-enabled-{peer}.json"));
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        peer_cache_persist_local_addrs: true,
        ..DiscoveryConfig::default()
    };
    let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
    record_seen_peer_addr(&cfg, &peer, &addr);
    let entries = load_entries(&cfg);
    let _ = fs::remove_file(&cfg.peer_cache_path);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].addr_kind, CachedDialAddrKind::LocalSession);
}
