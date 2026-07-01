#[cfg(test)]
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

use super::discovery::DiscoveryConfig;
use crate::platform::{DesktopPlatformRuntime, NodeStorage};

const CACHE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, Default)]
struct PeerCacheFile {
    #[serde(default = "default_cache_version")]
    version: u32,
    #[serde(default)]
    entries: Vec<CachedPeerAddr>,
    /// Backward-compatible read path for pre-Phase-5 caches. Bare addrs are ignored.
    #[serde(default)]
    addrs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedPeerAddr {
    pub peer_id: String,
    pub addr: String,
    #[serde(default)]
    pub last_seen_unix_secs: u64,
    #[serde(default)]
    pub failures: u32,
}

pub fn load_last_addrs(cfg: &DiscoveryConfig, limit: usize) -> Vec<Multiaddr> {
    load_last_addrs_with_storage(cfg, limit, &DesktopPlatformRuntime::default())
}

pub fn load_last_addrs_with_storage(
    cfg: &DiscoveryConfig,
    limit: usize,
    storage: &dyn NodeStorage,
) -> Vec<Multiaddr> {
    load_entries_with_storage(cfg, storage)
        .into_iter()
        .filter_map(|entry| entry.addr.parse::<Multiaddr>().ok())
        .take(limit)
        .collect()
}

pub fn load_entries(cfg: &DiscoveryConfig) -> Vec<CachedPeerAddr> {
    load_entries_with_storage(cfg, &DesktopPlatformRuntime::default())
}

pub fn load_entries_with_storage(
    cfg: &DiscoveryConfig,
    storage: &dyn NodeStorage,
) -> Vec<CachedPeerAddr> {
    let raw = match storage.read(&cfg.peer_cache_path) {
        Ok(Some(v)) => v,
        Ok(None) | Err(_) => return Vec::new(),
    };
    let raw = match String::from_utf8(raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let parsed: PeerCacheFile = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let _cache_version = parsed.version;
    let mut entries = parsed.entries;
    for addr in parsed.addrs {
        let peer = addr
            .parse::<Multiaddr>()
            .ok()
            .and_then(|parsed_addr| extract_last_p2p_peer_id(&parsed_addr));
        if let Some(peer) = peer {
            entries.push(CachedPeerAddr {
                peer_id: peer.to_string(),
                addr,
                last_seen_unix_secs: 0,
                failures: 0,
            });
        }
    }

    let now = now_unix_secs();
    let max_age = cfg.peer_cache_max_age_secs;
    let max_failures = cfg.peer_cache_max_failures;
    entries
        .into_iter()
        .filter(|entry| is_valid_cache_entry(entry, now, max_age, max_failures))
        .take(cfg.peer_cache_max_entries)
        .collect()
}

pub fn record_seen_peer_addr(cfg: &DiscoveryConfig, peer: &PeerId, addr: &Multiaddr) {
    record_seen_peer_addr_with_storage(cfg, peer, addr, &DesktopPlatformRuntime::default());
}

pub fn record_seen_peer_addr_with_storage(
    cfg: &DiscoveryConfig,
    peer: &PeerId,
    addr: &Multiaddr,
    storage: &dyn NodeStorage,
) {
    let Some(cache_addr) = normalize_peer_addr(peer, addr) else {
        return;
    };
    if !is_cacheable_peer_addr(&cache_addr, Some(peer)) {
        return;
    }

    let mut entries = load_entries_with_storage(cfg, storage);
    let addr_s = cache_addr.to_string();
    let peer_s = peer.to_string();
    entries.retain(|entry| entry.addr != addr_s && entry.peer_id != peer_s);
    entries.insert(
        0,
        CachedPeerAddr {
            peer_id: peer_s,
            addr: addr_s,
            last_seen_unix_secs: now_unix_secs(),
            failures: 0,
        },
    );
    entries.truncate(cfg.peer_cache_max_entries);
    write_entries_with_storage(&cfg.peer_cache_path, entries, storage);
}

pub fn record_peer_addr_failure(cfg: &DiscoveryConfig, peer: &PeerId) {
    record_peer_addr_failure_with_storage(cfg, peer, &DesktopPlatformRuntime::default());
}

pub fn record_peer_addr_failure_with_storage(
    cfg: &DiscoveryConfig,
    peer: &PeerId,
    storage: &dyn NodeStorage,
) {
    let mut entries = load_entries_with_storage(cfg, storage);
    let peer_s = peer.to_string();
    for entry in &mut entries {
        if entry.peer_id == peer_s {
            entry.failures = entry.failures.saturating_add(1);
        }
    }
    entries.retain(|entry| {
        cfg.peer_cache_max_failures == 0 || entry.failures < cfg.peer_cache_max_failures
    });
    write_entries_with_storage(&cfg.peer_cache_path, entries, storage);
}

pub fn is_cacheable_peer_addr(addr: &Multiaddr, expected_peer: Option<&PeerId>) -> bool {
    let Some(peer) = extract_last_p2p_peer_id(addr) else {
        return false;
    };
    if expected_peer.is_some_and(|expected| expected != &peer) {
        return false;
    }

    let mut has_reachable_transport = false;
    for protocol in addr.iter() {
        match protocol {
            Protocol::Ip4(ip) => {
                if ip.is_unspecified() {
                    return false;
                }
                has_reachable_transport = true;
            }
            Protocol::Ip6(ip) => {
                if ip.is_unspecified() {
                    return false;
                }
                has_reachable_transport = true;
            }
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) => {
                has_reachable_transport = true;
            }
            Protocol::Dnsaddr(_) => {
                return false;
            }
            _ => {}
        }
    }

    has_reachable_transport
}

pub fn normalize_peer_addr(peer: &PeerId, addr: &Multiaddr) -> Option<Multiaddr> {
    if is_cacheable_peer_addr(addr, Some(peer)) {
        return Some(addr.clone());
    }

    if contains_any_p2p(addr) || !has_reachable_transport(addr) || has_unspecified_ip(addr) {
        return None;
    }

    Some(addr.clone().with(Protocol::P2p(peer.to_owned())))
}

#[cfg(test)]
fn load_raw_entries_for_tests(path: impl AsRef<std::path::Path>) -> Vec<CachedPeerAddr> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<PeerCacheFile>(&raw)
        .map(|file| file.entries)
        .unwrap_or_default()
}

fn write_entries_with_storage(
    path: &str,
    entries: Vec<CachedPeerAddr>,
    storage: &dyn NodeStorage,
) {
    let payload = PeerCacheFile {
        version: CACHE_VERSION,
        entries,
        addrs: Vec::new(),
    };
    if let Ok(text) = serde_json::to_string_pretty(&payload) {
        let _ = storage.write_public(path, text.as_bytes());
    }
}

fn is_valid_cache_entry(
    entry: &CachedPeerAddr,
    now: u64,
    max_age_secs: u64,
    max_failures: u32,
) -> bool {
    let Ok(peer) = entry.peer_id.parse::<PeerId>() else {
        return false;
    };
    let Ok(addr) = entry.addr.parse::<Multiaddr>() else {
        return false;
    };
    if !is_cacheable_peer_addr(&addr, Some(&peer)) {
        return false;
    }
    if max_age_secs > 0
        && entry.last_seen_unix_secs > 0
        && now.saturating_sub(entry.last_seen_unix_secs) > max_age_secs
    {
        return false;
    }
    if max_failures > 0 && entry.failures >= max_failures {
        return false;
    }
    true
}

fn extract_last_p2p_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    let mut out = None;
    for protocol in addr.iter() {
        if let Protocol::P2p(peer) = protocol {
            out = Some(peer);
        }
    }
    out
}

fn contains_any_p2p(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::P2p(_)))
}

fn has_reachable_transport(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| {
        matches!(
            protocol,
            Protocol::Ip4(_)
                | Protocol::Ip6(_)
                | Protocol::Dns(_)
                | Protocol::Dns4(_)
                | Protocol::Dns6(_)
        )
    })
}

fn has_unspecified_ip(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| match protocol {
        Protocol::Ip4(ip) => ip.is_unspecified(),
        Protocol::Ip6(ip) => ip.is_unspecified(),
        _ => false,
    })
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn default_cache_version() -> u32 {
    CACHE_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn records_identity_bound_addr() {
        let peer = PeerId::random();
        let path = std::env::temp_dir().join(format!("p2p-cache-{peer}.json"));
        let cfg = DiscoveryConfig {
            peer_cache_path: path.to_string_lossy().to_string(),
            ..DiscoveryConfig::default()
        };
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        record_seen_peer_addr(&cfg, &peer, &addr);
        let entries = load_raw_entries_for_tests(&cfg.peer_cache_path);
        let _ = fs::remove_file(&cfg.peer_cache_path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].peer_id, peer.to_string());
        assert!(entries[0].addr.ends_with(&format!("/p2p/{peer}")));
    }
}
