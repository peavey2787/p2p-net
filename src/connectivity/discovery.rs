//! Discovery configuration and peer-cache policy for the standalone node.

use serde::{Deserialize, Serialize};

use super::rendezvous::RendezvousConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    /// On-disk peer cache path. Entries are validated identity-bound multiaddrs.
    pub peer_cache_path: String,
    /// Maximum number of cached peer addresses to keep.
    pub peer_cache_max_entries: usize,
    /// Drop cached peers older than this many seconds. Set to 0 to disable age eviction.
    pub peer_cache_max_age_secs: u64,
    /// Drop cached peers after this many recorded failures. Set to 0 to disable failure eviction.
    pub peer_cache_max_failures: u32,
    /// Public bootstrap seed peers. Full `/p2p/<PeerId>` multiaddrs are required.
    pub bootstrap_seed_peers: Vec<String>,
    /// Rendezvous peers. Full `/p2p/<PeerId>` multiaddrs are required. When the
    /// rendezvous client is enabled, these are used for registration/discovery.
    pub rendezvous_peers: Vec<String>,
    /// libp2p Rendezvous client/server controls. Disabled by default.
    #[serde(default)]
    pub rendezvous: RendezvousConfig,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            peer_cache_path: ".p2p-net-peer-cache.json".to_string(),
            peer_cache_max_entries: 64,
            peer_cache_max_age_secs: 30 * 24 * 60 * 60,
            peer_cache_max_failures: 3,
            bootstrap_seed_peers: Vec::new(),
            rendezvous_peers: Vec::new(),
            rendezvous: RendezvousConfig::default(),
        }
    }
}

impl DiscoveryConfig {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if self.peer_cache_path.trim().is_empty() {
            return Err(config_error("discovery.peer_cache_path must not be empty"));
        }
        if self.peer_cache_max_entries == 0 {
            return Err(config_error(
                "discovery.peer_cache_max_entries must be at least 1",
            ));
        }
        self.rendezvous.validate()?;
        Ok(())
    }
}

fn config_error(reason: impl Into<String>) -> crate::common::error::NetError {
    crate::common::error::NetError::Config {
        path: "<config>".to_string(),
        reason: reason.into(),
    }
}
