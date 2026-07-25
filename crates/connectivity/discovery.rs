//! Discovery configuration and peer-cache policy for the standalone node.

use crate::common::error::config_error;
use libp2p_rendezvous as rendezvous;
use serde::{Deserialize, Serialize};

use super::dht::DhtDiscoveryConfig;
use super::namespace::{DiscoveryNamespace, DiscoveryNamespaceConfig};
use super::public_fallback::PublicBootstrapConfig;
use super::relay_discovery::RelayDiscoveryPolicy;
use super::rendezvous::RendezvousConfig;

const APPLICATION_COMPATIBILITY_CONTEXT: &str = "p2p-net.application.compatibility.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    /// On-disk peer cache path. Entries are validated identity-bound multiaddrs.
    pub peer_cache_path: String,
    /// Maximum number of cached peer addresses to keep.
    pub peer_cache_max_entries: usize,
    /// Legacy maximum age for any cached dialable address. Set to 0 to disable this global ceiling.
    pub peer_cache_max_age_secs: u64,
    /// Maximum age for peer identity memory. Addresses can expire while identities remain known.
    pub peer_identity_max_age_secs: u64,
    /// Maximum age for public/NAT direct dial addresses. These are short-lived because NAT mappings and mobile IPs churn quickly.
    pub peer_cache_public_addr_max_age_secs: u64,
    /// Maximum age for relayed dial addresses when no exact relay reservation expiry is known.
    pub peer_cache_relay_addr_max_age_secs: u64,
    /// Persist local/LAN/loopback addresses across process restarts. Disabled by default; local addrs are session-only.
    pub peer_cache_persist_local_addrs: bool,
    /// Maximum age for persisted local/LAN/loopback addresses when explicitly enabled. Set to 0 to disable age eviction.
    pub peer_cache_local_addr_max_age_secs: u64,
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
    /// Public bootstrap/rendezvous/relay fallback and auto-connect policy. Enabled by default for consumer app mode; disable it for private-infrastructure-only operation.
    #[serde(default)]
    pub public_bootstrap: PublicBootstrapConfig,
    /// Kademlia provider-record discovery for hashed app namespaces.
    #[serde(default)]
    pub dht: DhtDiscoveryConfig,
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
            peer_identity_max_age_secs: 30 * 24 * 60 * 60,
            peer_cache_public_addr_max_age_secs: 10 * 60,
            peer_cache_relay_addr_max_age_secs: 2 * 60 * 60,
            peer_cache_persist_local_addrs: false,
            peer_cache_local_addr_max_age_secs: 0,
            peer_cache_max_failures: 3,
            bootstrap_seed_peers: Vec::new(),
            rendezvous_peers: Vec::new(),
            namespace: DiscoveryNamespaceConfig::default(),
            public_bootstrap: PublicBootstrapConfig::default(),
            dht: DhtDiscoveryConfig::default(),
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
        if self.peer_identity_max_age_secs == 0 {
            return Err(config_error(
                "discovery.peer_identity_max_age_secs must be at least 1",
            ));
        }
        if self.peer_cache_public_addr_max_age_secs == 0 {
            return Err(config_error(
                "discovery.peer_cache_public_addr_max_age_secs must be at least 1",
            ));
        }
        if self.peer_cache_relay_addr_max_age_secs == 0 {
            return Err(config_error(
                "discovery.peer_cache_relay_addr_max_age_secs must be at least 1",
            ));
        }
        self.namespace.validate()?;
        self.public_bootstrap.validate()?;
        self.dht.validate()?;
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

    /// Identify protocol version used as a connection-local application
    /// compatibility proof.
    ///
    /// The value contains only a domain-separated hash of the complete,
    /// canonical namespace set. It lets the receiving side verify an inbound
    /// direct or relayed connection before gossip mesh formation without
    /// exposing readable discovery tags.
    pub fn application_protocol_version(
        &self,
        network_id: u32,
    ) -> Result<String, crate::common::error::NetError> {
        let mut namespaces = self.rendezvous_namespaces(network_id)?;
        namespaces.sort_unstable();
        namespaces.dedup();

        let mut hasher = blake3::Hasher::new();
        hasher.update(APPLICATION_COMPATIBILITY_CONTEXT.as_bytes());
        hasher.update(&network_id.to_be_bytes());
        for namespace in namespaces {
            hasher.update(&[0]);
            hasher.update(namespace.as_bytes());
        }
        let fingerprint = hasher.finalize().to_hex();
        Ok(format!("/p2p-net/net-{network_id}/app-{fingerprint}/1.0.0"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_protocol_proves_exact_namespace_set_without_leaking_tag() {
        let mut config = DiscoveryConfig::default();
        config.namespace.tags = vec!["private desktop group".to_string()];

        let protocol = config.application_protocol_version(7).expect("protocol");

        assert!(protocol.starts_with("/p2p-net/net-7/app-"));
        assert!(!protocol.contains("private"));
        assert_eq!(
            protocol,
            config.application_protocol_version(7).expect("stable")
        );

        config.namespace.tags = vec!["different group".to_string()];
        assert_ne!(
            protocol,
            config.application_protocol_version(7).expect("different")
        );
    }

    #[test]
    fn application_protocol_is_independent_of_tag_order() {
        let mut first = DiscoveryConfig::default();
        first.namespace.tags = vec!["alpha".to_string(), "beta".to_string()];
        let mut second = first.clone();
        second.namespace.tags.reverse();

        assert_eq!(
            first.application_protocol_version(1).expect("first"),
            second.application_protocol_version(1).expect("second")
        );
    }
}
