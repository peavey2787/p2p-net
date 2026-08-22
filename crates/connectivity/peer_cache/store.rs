use std::collections::{HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use libp2p::{Multiaddr, PeerId};

use super::addr_policy::{
    classify_dialable_addr, extract_last_p2p_peer_id, inferred_expiry_secs,
    is_persistable_dialable_addr_kind, is_valid_cache_entry, normalize_entry_kind,
    normalize_peer_addr,
};
use super::model::{CachedPeerAddr, CachedPeerIdentity, PeerCacheFile, CACHE_VERSION};
use crate::connectivity::discovery::DiscoveryConfig;
use crate::platform::{DesktopPlatformRuntime, NodeStorage};

const MAX_PENDING_CACHE_MUTATIONS: usize = 4096;

#[derive(Debug)]
enum PeerCacheMutation {
    Seen {
        peer: PeerId,
        addr: Multiaddr,
        expires_unix_secs: Option<u64>,
    },
    Failure { peer: PeerId },
}

/// Runtime-owned, best-effort peer-cache write coalescer. Hot discovery/event
/// paths enqueue mutations and the node flushes them periodically, so a burst of
/// DHT/Identify/Rendezvous activity performs one cache read/parse/write cycle
/// instead of one full JSON rewrite per discovered address.
#[derive(Debug, Default)]
pub struct PeerCacheWriteBatch {
    pending: VecDeque<PeerCacheMutation>,
    pending_seen: HashSet<(PeerId, Multiaddr, Option<u64>)>,
}

impl PeerCacheWriteBatch {
    pub fn record_seen(&mut self, peer: PeerId, addr: Multiaddr) {
        self.record_seen_with_expiry(peer, addr, None);
    }

    pub fn record_seen_with_expiry(
        &mut self,
        peer: PeerId,
        addr: Multiaddr,
        expires_unix_secs: Option<u64>,
    ) {
        let key = (peer, addr.clone(), expires_unix_secs);
        if !self.pending_seen.insert(key) {
            return;
        }
        self.push(PeerCacheMutation::Seen {
            peer,
            addr,
            expires_unix_secs,
        });
    }

    pub fn record_failure(&mut self, peer: PeerId) {
        self.push(PeerCacheMutation::Failure { peer });
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn flush(&mut self, cfg: &DiscoveryConfig, storage: &dyn NodeStorage) {
        if self.pending.is_empty() {
            return;
        }
        let mutations = std::mem::take(&mut self.pending);
        self.pending_seen.clear();
        apply_mutations_with_storage(cfg, mutations, storage);
    }

    fn push(&mut self, mutation: PeerCacheMutation) {
        if self.pending.len() >= MAX_PENDING_CACHE_MUTATIONS {
            if let Some(PeerCacheMutation::Seen {
                peer,
                addr,
                expires_unix_secs,
            }) = self.pending.pop_front()
            {
                self.pending_seen.remove(&(peer, addr, expires_unix_secs));
            }
        }
        self.pending.push_back(mutation);
    }
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

pub fn record_peer_addr_failure(cfg: &DiscoveryConfig, peer: &PeerId) {
    record_peer_addr_failure_with_storage(cfg, peer, &DesktopPlatformRuntime::default());
}

pub fn record_peer_addr_failure_with_storage(
    cfg: &DiscoveryConfig,
    peer: &PeerId,
    storage: &dyn NodeStorage,
) {
    let mut batch = PeerCacheWriteBatch::default();
    batch.record_failure(*peer);
    batch.flush(cfg, storage);
}

fn record_seen_peer_addr_inner(
    cfg: &DiscoveryConfig,
    peer: &PeerId,
    addr: &Multiaddr,
    expires_unix_secs: Option<u64>,
    storage: &dyn NodeStorage,
) {
    let mut batch = PeerCacheWriteBatch::default();
    batch.record_seen_with_expiry(*peer, addr.clone(), expires_unix_secs);
    batch.flush(cfg, storage);
}

fn apply_mutations_with_storage(
    cfg: &DiscoveryConfig,
    mutations: VecDeque<PeerCacheMutation>,
    storage: &dyn NodeStorage,
) {
    let now = now_unix_secs();
    let file = read_cache_file_with_storage(cfg, storage).unwrap_or_default();
    let mut identities = valid_identities_from_file(cfg, &file, now);
    let mut entries = valid_dialable_entries_from_file(cfg, &file, now);

    for mutation in mutations {
        match mutation {
            PeerCacheMutation::Seen { peer, addr, expires_unix_secs } => {
                upsert_identity(&mut identities, &peer, now);
                let Some(cache_addr) = normalize_peer_addr(&peer, &addr) else {
                    continue;
                };
                let Some(addr_kind) = classify_dialable_addr(&cache_addr) else {
                    continue;
                };
                if !is_persistable_dialable_addr_kind(cfg, addr_kind) {
                    continue;
                }

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
                        expires_unix_secs: expires_unix_secs
                            .or_else(|| inferred_expiry_secs(cfg, addr_kind, now)),
                    },
                );
            }
            PeerCacheMutation::Failure { peer } => {
                let peer_s = peer.to_string();
                for entry in &mut entries {
                    if entry.peer_id == peer_s {
                        entry.failures = entry.failures.saturating_add(1);
                    }
                }
                entries.retain(|entry| {
                    cfg.peer_cache_max_failures == 0
                        || entry.failures < cfg.peer_cache_max_failures
                });
            }
        }
    }

    identities.truncate(cfg.peer_cache_max_entries);
    entries.truncate(cfg.peer_cache_max_entries);
    write_entries_with_storage(&cfg.peer_cache_path, identities, entries, storage);
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
    if let Ok(text) = serde_json::to_string(&payload) {
        let _ = storage.write_public(path, text.as_bytes());
    }
}

fn valid_dialable_entries_from_file(
    cfg: &DiscoveryConfig,
    file: &PeerCacheFile,
    now: u64,
) -> Vec<CachedPeerAddr> {
    // Version 2 accidentally retained unrelated public DHT/Identify peers as
    // application peers. Discard it once instead of redialing polluted
    // infrastructure entries forever after the retention bug is fixed.
    if file.version < CACHE_VERSION {
        return Vec::new();
    }
    let mut entries = Vec::new();
    entries.extend(file.dialable_addrs.clone());
    entries.extend(file.entries.clone());
    entries.extend(older_format_entries(file));

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

fn older_format_entries(file: &PeerCacheFile) -> Vec<CachedPeerAddr> {
    file.addrs
        .iter()
        .filter_map(|addr| {
            let peer = addr
                .parse::<Multiaddr>()
                .ok()
                .and_then(|parsed_addr| extract_last_p2p_peer_id(&parsed_addr))?;
            Some(CachedPeerAddr {
                peer_id: peer.to_string(),
                addr: addr.clone(),
                last_seen_unix_secs: 0,
                failures: 0,
                addr_kind: Default::default(),
                expires_unix_secs: None,
            })
        })
        .collect()
}

fn valid_identities_from_file(
    cfg: &DiscoveryConfig,
    file: &PeerCacheFile,
    now: u64,
) -> Vec<CachedPeerIdentity> {
    if file.version < CACHE_VERSION {
        return Vec::new();
    }
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

fn is_valid_identity_entry(cfg: &DiscoveryConfig, identity: &CachedPeerIdentity, now: u64) -> bool {
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

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
