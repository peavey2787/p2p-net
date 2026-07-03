//! On-disk peer memory split into durable identities and short-lived dialable addresses.

mod addr_policy;
mod model;
mod store;

#[cfg(test)]
mod tests;

pub use self::addr_policy::{
    classify_dialable_addr, is_cacheable_peer_addr, is_persistable_dialable_peer_addr,
    normalize_peer_addr,
};
pub use self::model::{CachedDialAddrKind, CachedPeerAddr, CachedPeerIdentity};
pub use self::store::{
    load_entries, load_entries_with_storage, load_identities, load_identities_with_storage,
    load_last_addrs, load_last_addrs_with_storage, record_peer_addr_failure,
    record_peer_addr_failure_with_storage, record_seen_peer_addr,
    record_seen_peer_addr_with_expiry, record_seen_peer_addr_with_expiry_with_storage,
    record_seen_peer_addr_with_storage,
};
