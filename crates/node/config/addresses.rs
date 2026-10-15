//! Parsed multiaddr views of the address lists in [`NodeConfig`].

use libp2p::Multiaddr;

use crate::common::error::NetError;

use super::super::config_validation::parse_multiaddrs;
use super::NodeConfig;

impl NodeConfig {
    pub fn parsed_listen_addresses(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs("listen_addresses", &self.listen_addresses)
    }

    /// Return only configured listen addresses whose transport listener is enabled.
    pub fn enabled_listen_addresses(&self) -> Result<Vec<Multiaddr>, NetError> {
        Ok(self
            .parsed_listen_addresses()?
            .into_iter()
            .filter(|addr| self.listeners.allows(addr))
            .collect())
    }

    pub fn parsed_bootstrap_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs("bootstrap_peers", &self.bootstrap_peers)
    }

    pub fn parsed_external_addresses(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs("external_addresses", &self.external_addresses)
    }

    pub fn parsed_relay_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs("relay_peers", &self.relay_peers)
    }

    pub fn parsed_bootstrap_seed_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.bootstrap_seed_peers",
            &self.discovery.bootstrap_seed_peers,
        )
    }

    pub fn parsed_public_bootstrap_seed_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.public_bootstrap.bootstrap_seed_peers",
            &self.discovery.public_bootstrap.bootstrap_seed_peers,
        )
    }

    pub fn parsed_public_rendezvous_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.public_bootstrap.rendezvous_peers",
            &self.discovery.public_bootstrap.rendezvous_peers,
        )
    }

    pub fn parsed_public_relay_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.public_bootstrap.relay_peers",
            &self.discovery.public_bootstrap.relay_peers,
        )
    }

    pub fn parsed_rendezvous_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.rendezvous_peers",
            &self.discovery.rendezvous_peers,
        )
    }
}
