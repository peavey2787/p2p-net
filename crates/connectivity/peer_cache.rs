//! On-disk peer memory split into durable identities and short-lived dialable addresses.

mod addr_policy;
mod model;
mod store;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

pub use self::addr_policy::{
    classify_dialable_addr, is_cacheable_peer_addr, is_persistable_dialable_peer_addr,
    normalize_peer_addr,
};
pub use self::model::{CachedDialAddrKind, CachedPeerAddr, CachedPeerIdentity};
#[cfg(not(target_arch = "wasm32"))]
pub use self::store::{
    load_entries, load_identities, load_last_addrs, record_peer_addr_failure,
    record_seen_peer_addr, record_seen_peer_addr_with_expiry,
};
pub use self::store::{
    load_entries_with_storage, load_identities_with_storage, load_last_addrs_with_storage,
    record_peer_addr_failure_with_storage, record_seen_peer_addr_with_expiry_with_storage,
    record_seen_peer_addr_with_storage, PeerCacheWriteBatch,
};
