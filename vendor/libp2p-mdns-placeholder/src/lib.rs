//! Policy placeholder for p2p-net.
//!
//! p2p-net does not include LAN multicast discovery/mDNS. This placeholder keeps
//! the libp2p meta-crate's optional mDNS dependency from locking its DNS parser
//! dependency when the feature is not enabled by p2p-net.

#[doc(hidden)]
pub struct UpstreamMdnsDisabled;
