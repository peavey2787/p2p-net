#[cfg(test)]
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::{SystemTime, UNIX_EPOCH};

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

use super::addr::{has_reachable_transport, has_unspecified_ip};
use super::discovery::DiscoveryConfig;
use crate::platform::{DesktopPlatformRuntime, NodeStorage};

const CACHE_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PeerCacheFile {
    #[serde(default = "default_cache_version")]
    version: u32,
    /// Long-lived identity memory. These records remember that a PeerId exists,
    /// but are never treated as proof that an address is still dialable.
    #[serde(default)]
    identities: Vec<CachedPeerIdentity>,
    /// Short-lived dialable address memory. Freshness depends on the address kind.
    #[serde(default)]
    dialable_addrs: Vec<CachedPeerAddr>,
    /// Backward-compatible read/write path for pre-v2 cache readers.
    #[serde(default)]
    entries: Vec<CachedPeerAddr>,
    /// Backward-compatible read path for pre-Phase-5 caches. Bare addrs are ignored.
    #[serde(default)]
    addrs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedPeerIdentity {
    pub peer_id: String,
    #[serde(default)]
    pub last_seen_unix_secs: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CachedDialAddrKind {
    /// Public/NAT direct address. These expire quickly because public IPs and NAT mappings churn.
    PublicDirect,
    /// Circuit-relay address. These are valid only while the relay reservation is still fresh.
    RelayReservation,
    /// Loopback, private LAN, link-local, or otherwise local address. Session-only unless opted in.
    LocalSession,
}

impl Default for CachedDialAddrKind {
    fn default() -> Self {
        Self::PublicDirect
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedPeerAddr {
    pub peer_id: String,
    pub addr: String,
    #[serde(default)]
    pub last_seen_unix_secs: u64,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub addr_kind: CachedDialAddrKind,
    /// Optional hard expiry for the dialable address. Used for addresses learned
    /// from bounded resources such as relay reservations when a caller knows an
    /// explicit expiry. If absent, the kind-specific max age is applied.
    #[serde(default)]
    pub expires_unix_secs: Option<u64>,
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
    let Some(file) = read_cache_file_with_storage(cfg, storage) else {
        return Vec::new();
    };
    valid_dialable_entries_from_file(cfg, &file, now_unix_secs())
}

pub fn load_identities(cfg: &DiscoveryConfig) -> Vec<CachedPeerIdentity> {
    load_identities_with_storage(cfg, &DesktopPlatformRuntime::default())
}

pub fn load_identities_with_storage(
    cfg: &DiscoveryConfig,
    storage: &dyn NodeStorage,
) -> Vec<CachedPeerIdentity> {
    let Some(file) = read_cache_file_with_storage(cfg, storage) else {
        return Vec::new();
    };
    valid_identities_from_file(cfg, &file, now_unix_secs())
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
    record_seen_peer_addr_inner(cfg, peer, addr, None, storage);
}

pub fn record_seen_peer_addr_with_expiry(
    cfg: &DiscoveryConfig,
    peer: &PeerId,
    addr: &Multiaddr,
    expires_unix_secs: u64,
) {
    record_seen_peer_addr_with_expiry_with_storage(
        cfg,
        peer,
        addr,
        expires_unix_secs,
        &DesktopPlatformRuntime::default(),
    );
}

pub fn record_seen_peer_addr_with_expiry_with_storage(
    cfg: &DiscoveryConfig,
    peer: &PeerId,
    addr: &Multiaddr,
    expires_unix_secs: u64,
    storage: &dyn NodeStorage,
) {
    record_seen_peer_addr_inner(cfg, peer, addr, Some(expires_unix_secs), storage);
}

fn record_seen_peer_addr_inner(
    cfg: &DiscoveryConfig,
    peer: &PeerId,
    addr: &Multiaddr,
    expires_unix_secs: Option<u64>,
    storage: &dyn NodeStorage,
) {
    let now = now_unix_secs();
    let file = read_cache_file_with_storage(cfg, storage).unwrap_or_default();
    let mut identities = valid_identities_from_file(cfg, &file, now);
    upsert_identity(&mut identities, peer, now);

    let Some(cache_addr) = normalize_peer_addr(peer, addr) else {
        let entries = valid_dialable_entries_from_file(cfg, &file, now);
        write_entries_with_storage(&cfg.peer_cache_path, identities, entries, storage);
        return;
    };
    let Some(addr_kind) = classify_dialable_addr(&cache_addr) else {
        let entries = valid_dialable_entries_from_file(cfg, &file, now);
        write_entries_with_storage(&cfg.peer_cache_path, identities, entries, storage);
        return;
    };
    if !is_persistable_dialable_addr_kind(cfg, addr_kind) {
        // Local/LAN/loopback addresses are valid for the current session, but by
        // default they are deliberately not persisted for later startup dials.
        let entries = valid_dialable_entries_from_file(cfg, &file, now);
        write_entries_with_storage(&cfg.peer_cache_path, identities, entries, storage);
        return;
    }

    let mut entries = valid_dialable_entries_from_file(cfg, &file, now);
    let addr_s = cache_addr.to_string();
    let peer_s = peer.to_string();
    entries.retain(|entry| entry.addr != addr_s && entry.peer_id != peer_s);
    entries.insert(
        0,
        CachedPeerAddr {
            peer_id: peer_s,
            addr: addr_s,
            last_seen_unix_secs: now,
            failures: 0,
            addr_kind,
            expires_unix_secs: expires_unix_secs.or_else(|| inferred_expiry_secs(cfg, addr_kind, now)),
        },
    );
    entries.truncate(cfg.peer_cache_max_entries);
    write_entries_with_storage(&cfg.peer_cache_path, identities, entries, storage);
}

pub fn record_peer_addr_failure(cfg: &DiscoveryConfig, peer: &PeerId) {
    record_peer_addr_failure_with_storage(cfg, peer, &DesktopPlatformRuntime::default());
}

pub fn record_peer_addr_failure_with_storage(
    cfg: &DiscoveryConfig,
    peer: &PeerId,
    storage: &dyn NodeStorage,
) {
    let now = now_unix_secs();
    let file = read_cache_file_with_storage(cfg, storage).unwrap_or_default();
    let identities = valid_identities_from_file(cfg, &file, now);
    let mut entries = valid_dialable_entries_from_file(cfg, &file, now);
    let peer_s = peer.to_string();
    for entry in &mut entries {
        if entry.peer_id == peer_s {
            entry.failures = entry.failures.saturating_add(1);
        }
    }
    entries.retain(|entry| {
        cfg.peer_cache_max_failures == 0 || entry.failures < cfg.peer_cache_max_failures
    });
    write_entries_with_storage(&cfg.peer_cache_path, identities, entries, storage);
}

pub fn is_cacheable_peer_addr(addr: &Multiaddr, expected_peer: Option<&PeerId>) -> bool {
    let Some(peer) = extract_last_p2p_peer_id(addr) else {
        return false;
    };
    if expected_peer.is_some_and(|expected| expected != &peer) {
        return false;
    }

    if has_unspecified_ip(addr) || contains_dnsaddr(addr) {
        return false;
    }

    has_reachable_transport(addr)
}

pub fn is_persistable_dialable_peer_addr(cfg: &DiscoveryConfig, addr: &Multiaddr) -> bool {
    if !is_cacheable_peer_addr(addr, None) {
        return false;
    }
    classify_dialable_addr(addr)
        .map(|kind| is_persistable_dialable_addr_kind(cfg, kind))
        .unwrap_or(false)
}

pub fn classify_dialable_addr(addr: &Multiaddr) -> Option<CachedDialAddrKind> {
    if !has_reachable_transport(addr) || has_unspecified_ip(addr) || contains_dnsaddr(addr) {
        return None;
    }
    if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
    {
        return Some(CachedDialAddrKind::RelayReservation);
    }
    if contains_local_ip(addr) {
        return Some(CachedDialAddrKind::LocalSession);
    }
    Some(CachedDialAddrKind::PublicDirect)
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
        .map(|file| file.dialable_addrs)
        .unwrap_or_default()
}

fn read_cache_file_with_storage(
    cfg: &DiscoveryConfig,
    storage: &dyn NodeStorage,
) -> Option<PeerCacheFile> {
    let raw = match storage.read(&cfg.peer_cache_path) {
        Ok(Some(v)) => v,
        Ok(None) | Err(_) => return None,
    };
    let raw = String::from_utf8(raw).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_entries_with_storage(
    path: &str,
    identities: Vec<CachedPeerIdentity>,
    entries: Vec<CachedPeerAddr>,
    storage: &dyn NodeStorage,
) {
    let payload = PeerCacheFile {
        version: CACHE_VERSION,
        identities,
        // Keep both fields populated: v2 code reads dialable_addrs first, while
        // older diagnostic tools/tests that still inspect entries see the same data.
        dialable_addrs: entries.clone(),
        entries,
        addrs: Vec::new(),
    };
    if let Ok(text) = serde_json::to_string_pretty(&payload) {
        let _ = storage.write_public(path, text.as_bytes());
    }
}

fn valid_dialable_entries_from_file(
    cfg: &DiscoveryConfig,
    file: &PeerCacheFile,
    now: u64,
) -> Vec<CachedPeerAddr> {
    let _cache_version = file.version;
    let mut entries = Vec::new();
    entries.extend(file.dialable_addrs.clone());
    entries.extend(file.entries.clone());
    for addr in &file.addrs {
        let peer = addr
            .parse::<Multiaddr>()
            .ok()
            .and_then(|parsed_addr| extract_last_p2p_peer_id(&parsed_addr));
        if let Some(peer) = peer {
            entries.push(CachedPeerAddr {
                peer_id: peer.to_string(),
                addr: addr.clone(),
                last_seen_unix_secs: 0,
                failures: 0,
                addr_kind: CachedDialAddrKind::PublicDirect,
                expires_unix_secs: None,
            });
        }
    }

    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter_map(|mut entry| {
            normalize_entry_kind(&mut entry)?;
            if !seen.insert((entry.peer_id.clone(), entry.addr.clone())) {
                return None;
            }
            Some(entry)
        })
        .filter(|entry| is_valid_cache_entry(cfg, entry, now))
        .take(cfg.peer_cache_max_entries)
        .collect()
}

fn valid_identities_from_file(
    cfg: &DiscoveryConfig,
    file: &PeerCacheFile,
    now: u64,
) -> Vec<CachedPeerIdentity> {
    let mut identities = file.identities.clone();
    for entry in valid_dialable_entries_from_file(cfg, file, now) {
        identities.push(CachedPeerIdentity {
            peer_id: entry.peer_id,
            last_seen_unix_secs: entry.last_seen_unix_secs,
        });
    }

    let mut seen = std::collections::HashSet::new();
    identities
        .into_iter()
        .filter(|identity| is_valid_identity_entry(cfg, identity, now))
        .filter(|identity| seen.insert(identity.peer_id.clone()))
        .take(cfg.peer_cache_max_entries)
        .collect()
}

fn is_valid_cache_entry(cfg: &DiscoveryConfig, entry: &CachedPeerAddr, now: u64) -> bool {
    let Ok(peer) = entry.peer_id.parse::<PeerId>() else {
        return false;
    };
    let Ok(addr) = entry.addr.parse::<Multiaddr>() else {
        return false;
    };
    if !is_cacheable_peer_addr(&addr, Some(&peer)) {
        return false;
    }
    let Some(kind) = classify_dialable_addr(&addr) else {
        return false;
    };
    if !is_persistable_dialable_addr_kind(cfg, kind) {
        return false;
    }
    if let Some(expires) = entry.expires_unix_secs {
        if expires <= now {
            return false;
        }
    }
    if let Some(max_age_secs) = effective_dialable_max_age_secs(cfg, kind) {
        if entry.last_seen_unix_secs > 0
            && now.saturating_sub(entry.last_seen_unix_secs) > max_age_secs
        {
            return false;
        }
    }
    if cfg.peer_cache_max_failures > 0 && entry.failures >= cfg.peer_cache_max_failures {
        return false;
    }
    true
}

fn is_valid_identity_entry(
    cfg: &DiscoveryConfig,
    identity: &CachedPeerIdentity,
    now: u64,
) -> bool {
    if identity.peer_id.parse::<PeerId>().is_err() {
        return false;
    }
    if identity.last_seen_unix_secs > 0
        && now.saturating_sub(identity.last_seen_unix_secs) > cfg.peer_identity_max_age_secs
    {
        return false;
    }
    true
}

fn normalize_entry_kind(entry: &mut CachedPeerAddr) -> Option<()> {
    let addr = entry.addr.parse::<Multiaddr>().ok()?;
    entry.addr_kind = classify_dialable_addr(&addr)?;
    Some(())
}

fn upsert_identity(identities: &mut Vec<CachedPeerIdentity>, peer: &PeerId, now: u64) {
    let peer_s = peer.to_string();
    identities.retain(|identity| identity.peer_id != peer_s);
    identities.insert(
        0,
        CachedPeerIdentity {
            peer_id: peer_s,
            last_seen_unix_secs: now,
        },
    );
}

fn inferred_expiry_secs(
    cfg: &DiscoveryConfig,
    addr_kind: CachedDialAddrKind,
    now: u64,
) -> Option<u64> {
    effective_dialable_max_age_secs(cfg, addr_kind).map(|ttl| now.saturating_add(ttl))
}

fn effective_dialable_max_age_secs(
    cfg: &DiscoveryConfig,
    addr_kind: CachedDialAddrKind,
) -> Option<u64> {
    let kind_max = match addr_kind {
        CachedDialAddrKind::PublicDirect => Some(cfg.peer_cache_public_addr_max_age_secs),
        CachedDialAddrKind::RelayReservation => Some(cfg.peer_cache_relay_addr_max_age_secs),
        CachedDialAddrKind::LocalSession => {
            if cfg.peer_cache_local_addr_max_age_secs == 0 {
                None
            } else {
                Some(cfg.peer_cache_local_addr_max_age_secs)
            }
        }
    };
    min_nonzero(kind_max, cfg.peer_cache_max_age_secs)
}

fn min_nonzero(kind_max: Option<u64>, global_max: u64) -> Option<u64> {
    match (kind_max, global_max) {
        (None, 0) => None,
        (None, max) => Some(max),
        (Some(max), 0) => Some(max),
        (Some(kind), global) => Some(kind.min(global)),
    }
}

fn is_persistable_dialable_addr_kind(
    cfg: &DiscoveryConfig,
    addr_kind: CachedDialAddrKind,
) -> bool {
    match addr_kind {
        CachedDialAddrKind::PublicDirect | CachedDialAddrKind::RelayReservation => true,
        CachedDialAddrKind::LocalSession => cfg.peer_cache_persist_local_addrs,
    }
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

fn contains_dnsaddr(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::Dnsaddr(_)))
}

fn contains_local_ip(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| match protocol {
        Protocol::Ip4(ip) => is_local_ipv4(ip),
        Protocol::Ip6(ip) => is_local_ipv6(ip),
        _ => false,
    })
}

fn is_local_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn is_local_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
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
}
