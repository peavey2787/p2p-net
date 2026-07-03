use serde::{Deserialize, Serialize};

pub(super) const CACHE_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct PeerCacheFile {
    #[serde(default = "default_cache_version")]
    pub(super) version: u32,
    /// Long-lived identity memory. These records remember that a PeerId exists,
    /// but are never treated as proof that an address is still dialable.
    #[serde(default)]
    pub(super) identities: Vec<CachedPeerIdentity>,
    /// Short-lived dialable address memory. Freshness depends on the address kind.
    #[serde(default)]
    pub(super) dialable_addrs: Vec<CachedPeerAddr>,
    /// Backward-compatible read/write path for pre-v2 cache readers.
    #[serde(default)]
    pub(super) entries: Vec<CachedPeerAddr>,
    /// Backward-compatible read path for pre-Phase-5 caches. Bare addrs are ignored.
    #[serde(default)]
    pub(super) addrs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedPeerIdentity {
    pub peer_id: String,
    #[serde(default)]
    pub last_seen_unix_secs: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CachedDialAddrKind {
    /// Public/NAT direct address. These expire quickly because public IPs and NAT mappings churn.
    #[default]
    PublicDirect,
    /// Circuit-relay address. These are valid only while the relay reservation is still fresh.
    RelayReservation,
    /// Loopback, private LAN, link-local, or otherwise local address. Session-only unless opted in.
    LocalSession,
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

fn default_cache_version() -> u32 {
    CACHE_VERSION
}
