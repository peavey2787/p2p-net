//! Public discovery fallback policy.
//!
//! Consumer app mode enables public fallback by default so two fresh nodes can
//! find a bootstrap path without manual peer configuration. Operator and power
//! user deployments can disable this policy or replace the bundled public peers
//! with private bootstrap, rendezvous, mediator, and relay infrastructure.

use crate::common::error::config_error;
use serde::{Deserialize, Serialize};

/// Built-in public bootstrap seed peers used by the default consumer config.
///
/// Applications that ship their own public infrastructure should replace these
/// with distribution-owned peers. Power users can also override them with
/// explicit `bootstrap_peers` / `discovery.bootstrap_seed_peers` entries or set
/// [`PublicFallbackMode::Disabled`] for private-infrastructure-only operation.
pub const DEFAULT_PUBLIC_BOOTSTRAP_SEED_PEERS: &[&str] = &[
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa",
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt",
];

/// Built-in public app rendezvous candidates used by the default consumer config.
///
/// The shared library does not currently ship a project-operated rendezvous
/// fleet. App distributions that want true run-two-fresh-installs discovery
/// should publish real rendezvous DNSADDR entries here or override this list in
/// their app config. Keeping the list empty avoids silently pretending a
/// rendezvous service exists when none is operated by this repo.
pub const DEFAULT_PUBLIC_RENDEZVOUS_PEERS: &[&str] = &[];

/// Built-in public relay candidates used by the default consumer config.
///
/// The shared library does not currently ship a project-operated relay fleet.
/// App distributions that want default NAT-to-NAT relay connectivity should
/// publish real relay DNSADDR entries here or override this list in their app
/// config. Keeping the list empty avoids silently pretending a relay service
/// exists when none is operated by this repo.
pub const DEFAULT_PUBLIC_RELAY_PEERS: &[&str] = &[];

/// When public discovery fallback may be used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicFallbackMode {
    /// Never use public bootstrap, rendezvous, or relay candidates.
    Disabled,
    /// Use public candidates only when operator-owned/cached candidates are empty.
    #[default]
    FallbackOnly,
    /// Always include public candidates after operator-owned/cached candidates.
    Always,
}

impl PublicFallbackMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::FallbackOnly => "fallback_only",
            Self::Always => "always",
        }
    }

    #[must_use]
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

/// Public bootstrap/rendezvous/relay fallback configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PublicBootstrapConfig {
    /// Public fallback mode. Defaults to consumer-friendly fallback-only mode.
    pub mode: PublicFallbackMode,
    /// Public DHT/bootstrap peers. Full `/p2p/<PeerId>` multiaddrs are required.
    pub bootstrap_seed_peers: Vec<String>,
    /// Public app rendezvous candidates. Full `/p2p/<PeerId>` multiaddrs are required.
    pub rendezvous_peers: Vec<String>,
    /// Public relay/mediator candidates. Full `/p2p/<PeerId>` multiaddrs are required.
    pub relay_peers: Vec<String>,
    /// Automatically dial peers discovered through app-namespace public discovery.
    /// This is network-layer connectivity only; it must not add trusted contacts.
    pub auto_connect_discovered_peers: bool,
}

impl Default for PublicBootstrapConfig {
    fn default() -> Self {
        Self {
            mode: PublicFallbackMode::FallbackOnly,
            bootstrap_seed_peers: DEFAULT_PUBLIC_BOOTSTRAP_SEED_PEERS
                .iter()
                .map(|peer| (*peer).to_string())
                .collect(),
            rendezvous_peers: DEFAULT_PUBLIC_RENDEZVOUS_PEERS
                .iter()
                .map(|peer| (*peer).to_string())
                .collect(),
            relay_peers: DEFAULT_PUBLIC_RELAY_PEERS
                .iter()
                .map(|peer| (*peer).to_string())
                .collect(),
            auto_connect_discovered_peers: true,
        }
    }
}

impl PublicBootstrapConfig {
    /// Private/operator mode disables public fallback and clears bundled public peers.
    #[must_use]
    pub fn private_infrastructure_only() -> Self {
        Self {
            mode: PublicFallbackMode::Disabled,
            bootstrap_seed_peers: Vec::new(),
            rendezvous_peers: Vec::new(),
            relay_peers: Vec::new(),
            auto_connect_discovered_peers: false,
        }
    }

    #[must_use]
    pub fn has_rendezvous_candidates(&self) -> bool {
        self.mode.is_enabled() && !self.rendezvous_peers.is_empty()
    }

    #[must_use]
    pub fn has_relay_candidates(&self) -> bool {
        self.mode.is_enabled() && !self.relay_peers.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if self.mode.is_enabled()
            && self.bootstrap_seed_peers.is_empty()
            && self.rendezvous_peers.is_empty()
            && self.relay_peers.is_empty()
        {
            return Err(config_error(
                "discovery.public_bootstrap mode is enabled but no public bootstrap, rendezvous, or relay peers are configured",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn bootstrap_decision(&self, owned_startup_candidates: usize) -> PublicFallbackDecision {
        let use_public = match self.mode {
            PublicFallbackMode::Disabled => false,
            PublicFallbackMode::FallbackOnly => owned_startup_candidates == 0,
            PublicFallbackMode::Always => true,
        } && !self.bootstrap_seed_peers.is_empty();

        PublicFallbackDecision::new(
            self.mode,
            use_public,
            if use_public {
                if matches!(self.mode, PublicFallbackMode::Always) {
                    "always"
                } else {
                    "no_operator_or_cached_startup_candidates"
                }
            } else {
                "not_used"
            },
        )
    }

    #[must_use]
    pub fn rendezvous_decision(
        &self,
        owned_rendezvous_candidates: usize,
    ) -> PublicFallbackDecision {
        let use_public = match self.mode {
            PublicFallbackMode::Disabled => false,
            PublicFallbackMode::FallbackOnly => owned_rendezvous_candidates == 0,
            PublicFallbackMode::Always => true,
        } && !self.rendezvous_peers.is_empty();

        PublicFallbackDecision::new(
            self.mode,
            use_public,
            if use_public {
                if matches!(self.mode, PublicFallbackMode::Always) {
                    "always"
                } else {
                    "no_operator_or_cached_rendezvous_candidates"
                }
            } else {
                "not_used"
            },
        )
    }

    #[must_use]
    pub fn relay_decision(&self, owned_selected_relays: usize) -> PublicFallbackDecision {
        let use_public = match self.mode {
            PublicFallbackMode::Disabled => false,
            PublicFallbackMode::FallbackOnly => owned_selected_relays == 0,
            PublicFallbackMode::Always => true,
        } && !self.relay_peers.is_empty();

        PublicFallbackDecision::new(
            self.mode,
            use_public,
            if use_public {
                if matches!(self.mode, PublicFallbackMode::Always) {
                    "always"
                } else {
                    "no_operator_cached_or_rendezvous_relay_candidates"
                }
            } else {
                "not_used"
            },
        )
    }
}

/// Decision output for observability and tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFallbackDecision {
    pub mode: PublicFallbackMode,
    pub used: bool,
    pub reason: String,
}

impl PublicFallbackDecision {
    #[must_use]
    pub fn new(mode: PublicFallbackMode, used: bool, reason: impl Into<String>) -> Self {
        Self {
            mode,
            used,
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_public_fallback_is_consumer_friendly() {
        let cfg = PublicBootstrapConfig::default();

        assert_eq!(cfg.mode, PublicFallbackMode::FallbackOnly);
        assert!(!cfg.bootstrap_seed_peers.is_empty());
        assert!(cfg.rendezvous_peers.is_empty());
        assert!(cfg.relay_peers.is_empty());
        assert!(cfg.auto_connect_discovered_peers);
        assert!(!cfg.bootstrap_decision(1).used);
        assert!(cfg.bootstrap_decision(0).used);
        assert!(!cfg.rendezvous_decision(0).used);
    }

    #[test]
    fn private_infrastructure_only_disables_public_fallback() {
        let cfg = PublicBootstrapConfig::private_infrastructure_only();

        assert_eq!(cfg.mode, PublicFallbackMode::Disabled);
        assert!(cfg.bootstrap_seed_peers.is_empty());
        assert!(cfg.rendezvous_peers.is_empty());
        assert!(cfg.relay_peers.is_empty());
        assert!(!cfg.auto_connect_discovered_peers);
        assert!(!cfg.bootstrap_decision(0).used);
    }
}
