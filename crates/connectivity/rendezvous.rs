//! libp2p Rendezvous client/server configuration and runtime bookkeeping.
//!
//! This crate uses the direct `libp2p-rendezvous` crate at `0.17.1` instead of the
//! `libp2p` meta-crate rendezvous feature. That keeps the server on the patched
//! implementation that includes per-peer and total registration caps.

use crate::common::error::config_error;
use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId};
use libp2p_rendezvous as rendezvous;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RendezvousConfig {
    /// Enable the rendezvous client behaviour. When enabled, configured `rendezvous_peers`
    /// can be used for registration and discovery.
    pub client_enabled: bool,
    /// Enable this node as a rendezvous point/server.
    pub server_enabled: bool,
    /// Application namespace for registration/discovery.
    pub namespace: String,
    /// Register this node with configured rendezvous peers once it has external/listen addresses.
    pub register: bool,
    /// Discover peers from configured rendezvous peers.
    pub discover: bool,
    /// TTL requested when registering with a rendezvous peer.
    pub register_ttl_secs: u64,
    /// Optional maximum peers returned per discover request. `0` means use the protocol default.
    pub discover_limit: u64,
    /// Server-side minimum accepted TTL.
    pub server_min_ttl_secs: u64,
    /// Server-side maximum accepted TTL.
    pub server_max_ttl_secs: u64,
    /// Server-side maximum active namespace registrations per peer.
    pub server_max_registrations_per_peer: usize,
    /// Server-side maximum active registrations across all peers.
    pub server_max_registrations_total: usize,
    /// Server-side cookie cache size.
    pub server_max_stored_cookies: usize,
}

impl Default for RendezvousConfig {
    fn default() -> Self {
        Self {
            client_enabled: false,
            server_enabled: false,
            namespace: "p2p-net".to_string(),
            register: true,
            discover: true,
            register_ttl_secs: rendezvous::DEFAULT_TTL,
            discover_limit: 64,
            server_min_ttl_secs: rendezvous::MIN_TTL,
            server_max_ttl_secs: rendezvous::MAX_TTL,
            server_max_registrations_per_peer: rendezvous::server::MAX_REGISTRATION_PEER,
            server_max_registrations_total: rendezvous::server::MAX_REGISTRATIONS_TOTAL,
            server_max_stored_cookies: rendezvous::server::COOKIES_CACHE_SIZE,
        }
    }
}

impl RendezvousConfig {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if self.namespace.trim().is_empty() {
            return Err(config_error(
                "discovery.rendezvous.namespace must not be empty",
            ));
        }
        self.namespace()?;
        if self.register_ttl_secs == 0 {
            return Err(config_error(
                "discovery.rendezvous.register_ttl_secs must be at least 1",
            ));
        }
        if self.server_min_ttl_secs == 0 {
            return Err(config_error(
                "discovery.rendezvous.server_min_ttl_secs must be at least 1",
            ));
        }
        if self.server_max_ttl_secs < self.server_min_ttl_secs {
            return Err(config_error(
                "discovery.rendezvous.server_max_ttl_secs must be >= server_min_ttl_secs",
            ));
        }
        if self.server_max_registrations_per_peer == 0 {
            return Err(config_error(
                "discovery.rendezvous.server_max_registrations_per_peer must be at least 1",
            ));
        }
        if self.server_max_registrations_total == 0 {
            return Err(config_error(
                "discovery.rendezvous.server_max_registrations_total must be at least 1",
            ));
        }
        if self.server_max_registrations_per_peer > self.server_max_registrations_total {
            return Err(config_error(
                "discovery.rendezvous.server_max_registrations_per_peer cannot exceed server_max_registrations_total",
            ));
        }
        if self.server_max_stored_cookies == 0 {
            return Err(config_error(
                "discovery.rendezvous.server_max_stored_cookies must be at least 1",
            ));
        }
        Ok(())
    }

    pub fn namespace(&self) -> Result<rendezvous::Namespace, crate::common::error::NetError> {
        rendezvous::Namespace::new(self.namespace.clone()).map_err(|err| {
            config_error(format!(
                "discovery.rendezvous.namespace `{}` is invalid: {err}",
                self.namespace
            ))
        })
    }

    pub fn discover_limit(&self) -> Option<u64> {
        (self.discover_limit > 0).then_some(self.discover_limit)
    }

    pub fn server_config(&self) -> rendezvous::server::Config {
        rendezvous::server::Config::default()
            .with_min_ttl(self.server_min_ttl_secs)
            .with_max_ttl(self.server_max_ttl_secs)
            .with_max_registration_per_peer(self.server_max_registrations_per_peer)
            .with_max_registration_total(self.server_max_registrations_total)
            .with_max_stored_cookies(self.server_max_stored_cookies)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RendezvousPeerNamespace {
    pub peer: PeerId,
    pub namespace: String,
}

impl RendezvousPeerNamespace {
    #[must_use]
    pub fn new(peer: PeerId, namespace: impl Into<String>) -> Self {
        Self {
            peer,
            namespace: namespace.into(),
        }
    }
}

#[derive(Debug, Default)]
pub struct RendezvousState {
    pub registered_with: HashSet<PeerId>,
    pub register_inflight: HashSet<PeerId>,
    pub discover_inflight: HashSet<PeerId>,
    pub registered_namespaces: HashSet<RendezvousPeerNamespace>,
    pub register_inflight_namespaces: HashSet<RendezvousPeerNamespace>,
    pub discover_inflight_namespaces: HashSet<RendezvousPeerNamespace>,
    pub discovered_peers: HashSet<PeerId>,
    pub cookies: HashMap<PeerId, rendezvous::Cookie>,
    pub cookies_by_namespace: HashMap<RendezvousPeerNamespace, rendezvous::Cookie>,
    pub register_attempts: usize,
    pub register_failures: usize,
    pub discover_attempts: usize,
    pub discover_failures: usize,
    pub server_registrations: usize,
    pub server_discoveries_served: usize,
    pub server_errors: usize,
}

impl RendezvousState {
    #[must_use]
    pub fn namespace_registration_count(&self) -> usize {
        self.registered_namespaces.len()
    }

    #[must_use]
    pub fn namespace_discover_inflight_count(&self) -> usize {
        self.discover_inflight_namespaces.len()
    }

    pub fn mark_register_inflight(&mut self, peer: PeerId, namespace: &str) {
        self.register_inflight.insert(peer);
        self.register_inflight_namespaces
            .insert(RendezvousPeerNamespace::new(peer, namespace));
    }

    pub fn mark_registered(&mut self, peer: PeerId, namespace: &str) {
        self.register_inflight.remove(&peer);
        self.registered_with.insert(peer);
        let key = RendezvousPeerNamespace::new(peer, namespace);
        self.register_inflight_namespaces.remove(&key);
        self.registered_namespaces.insert(key);
    }

    pub fn mark_register_failed(&mut self, peer: PeerId, namespace: &str) {
        self.register_inflight.remove(&peer);
        self.register_inflight_namespaces
            .remove(&RendezvousPeerNamespace::new(peer, namespace));
    }

    pub fn is_register_inflight(&self, peer: PeerId, namespace: &str) -> bool {
        self.register_inflight_namespaces
            .contains(&RendezvousPeerNamespace::new(peer, namespace))
    }

    pub fn is_registered(&self, peer: PeerId, namespace: &str) -> bool {
        self.registered_namespaces
            .contains(&RendezvousPeerNamespace::new(peer, namespace))
    }

    pub fn mark_discover_inflight(&mut self, peer: PeerId, namespace: &str) {
        self.discover_inflight.insert(peer);
        self.discover_inflight_namespaces
            .insert(RendezvousPeerNamespace::new(peer, namespace));
    }

    pub fn discover_cookie(&self, peer: PeerId, namespace: &str) -> Option<rendezvous::Cookie> {
        self.cookies_by_namespace
            .get(&RendezvousPeerNamespace::new(peer, namespace))
            .cloned()
            .or_else(|| self.cookies.get(&peer).cloned())
    }

    pub fn complete_discover_for_peer(
        &mut self,
        peer: PeerId,
        cookie: rendezvous::Cookie,
    ) -> Option<String> {
        self.discover_inflight.remove(&peer);
        self.cookies.insert(peer, cookie.clone());

        let key = self
            .discover_inflight_namespaces
            .iter()
            .find(|candidate| candidate.peer == peer)
            .cloned();
        if let Some(key) = key {
            self.discover_inflight_namespaces.remove(&key);
            self.cookies_by_namespace.insert(key.clone(), cookie);
            Some(key.namespace)
        } else {
            None
        }
    }

    pub fn fail_discover(&mut self, peer: PeerId, namespace: Option<&str>) {
        self.discover_inflight.remove(&peer);
        if let Some(namespace) = namespace {
            self.discover_inflight_namespaces
                .remove(&RendezvousPeerNamespace::new(peer, namespace));
        } else {
            self.discover_inflight_namespaces
                .retain(|candidate| candidate.peer != peer);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RendezvousActionPlan {
    pub register_attempts: usize,
    pub discover_attempts: usize,
    pub errors: Vec<String>,
}

pub fn peer_record_addrs(registration: &rendezvous::Registration) -> Vec<Multiaddr> {
    let peer = registration.record.peer_id();
    registration
        .record
        .addresses()
        .iter()
        .cloned()
        .map(|addr| {
            if addr
                .iter()
                .any(|protocol| matches!(protocol, libp2p::multiaddr::Protocol::P2p(_)))
            {
                addr
            } else {
                addr.with(libp2p::multiaddr::Protocol::P2p(peer))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendezvous_defaults_are_client_server_disabled() {
        let cfg = RendezvousConfig::default();
        assert!(!cfg.client_enabled);
        assert!(!cfg.server_enabled);
        cfg.validate().expect("default rendezvous config validates");
    }

    #[test]
    fn invalid_rendezvous_limits_fail_validation() {
        let bad_ttl = RendezvousConfig {
            server_min_ttl_secs: 10,
            server_max_ttl_secs: 5,
            ..RendezvousConfig::default()
        };
        assert!(bad_ttl.validate().is_err());

        let bad_regs = RendezvousConfig {
            server_max_registrations_per_peer: 2,
            server_max_registrations_total: 1,
            ..RendezvousConfig::default()
        };
        assert!(bad_regs.validate().is_err());
    }

    #[test]
    fn dynamic_namespace_validates() {
        let cfg = RendezvousConfig {
            namespace: "hydra-msg-testnet".to_string(),
            ..RendezvousConfig::default()
        };
        let ns = cfg.namespace().expect("namespace");
        assert_eq!(ns.to_string(), "hydra-msg-testnet");
    }
}
