//! Discovery configuration and peer-cache policy for the standalone node.

use libp2p_rendezvous as rendezvous;
use serde::{Deserialize, Serialize};

use super::namespace::{DiscoveryNamespace, DiscoveryNamespaceConfig};
use super::public_fallback::PublicBootstrapConfig;
use super::relay_discovery::RelayDiscoveryPolicy;
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
    /// Application discovery namespace derivation. Tags are hashed by default before publication.
    #[serde(default)]
    pub namespace: DiscoveryNamespaceConfig,
    /// Explicit public bootstrap/relay fallback. Disabled by default; operator-owned infra is preferred.
    #[serde(default)]
    pub public_bootstrap: PublicBootstrapConfig,
    /// Relay discovery/selection controls for lite/mobile nodes.
    #[serde(default)]
    pub relay_discovery: RelayDiscoveryPolicy,
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
            namespace: DiscoveryNamespaceConfig::default(),
            public_bootstrap: PublicBootstrapConfig::default(),
            relay_discovery: RelayDiscoveryPolicy::default(),
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
        self.namespace.validate()?;
        self.public_bootstrap.validate()?;
        self.relay_discovery.validate()?;
        self.rendezvous.validate()?;
        for ns in self.rendezvous_namespaces(0)? {
            rendezvous::Namespace::new(ns).map_err(|err| {
                config_error(format!(
                    "derived discovery namespace is invalid for rendezvous: {err}"
                ))
            })?;
        }
        Ok(())
    }

    /// Namespaces used by rendezvous/DHT discovery. When app tags are configured,
    /// derived hashed app namespaces replace the operator rendezvous namespace.
    /// If no tags are configured, `discovery.rendezvous.namespace` remains
    /// active for backward compatibility and operator-managed infrastructure.
    pub fn rendezvous_namespaces(
        &self,
        network_id: u32,
    ) -> Result<Vec<String>, crate::common::error::NetError> {
        if self.namespace.is_enabled() {
            return self
                .namespace
                .derived_namespaces(network_id)
                .map(|items| items.into_iter().map(|item| item.namespace).collect());
        }
        Ok(vec![self.rendezvous.namespace.clone()])
    }

    /// Metadata for configured app-level namespaces. Empty when the operator
    /// rendezvous namespace is in use.
    pub fn derived_app_namespaces(
        &self,
        network_id: u32,
    ) -> Result<Vec<DiscoveryNamespace>, crate::common::error::NetError> {
        self.namespace.derived_namespaces(network_id)
    }
}

fn config_error(reason: impl Into<String>) -> crate::common::error::NetError {
    crate::common::error::NetError::Config {
        path: "<config>".to_string(),
        reason: reason.into(),
    }
}
