//! Explicit public discovery fallback policy.
//!
//! Public fallback is intentionally opt-in. Production deployments should prefer
//! operator-owned bootstrap, rendezvous, mediator, and relay nodes. Public seeds
//! are used only when configured and allowed by policy.

use serde::{Deserialize, Serialize};

/// When public discovery fallback may be used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicFallbackMode {
    /// Never use public bootstrap or relay candidates.
    #[default]
    Disabled,
    /// Use public candidates only when operator-owned/cached candidates are empty.
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

/// Opt-in public bootstrap/relay fallback configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PublicBootstrapConfig {
    /// Public fallback mode. Defaults to disabled so there are no hidden public dependencies.
    pub mode: PublicFallbackMode,
    /// Public DHT/bootstrap peers. Full `/p2p/<PeerId>` multiaddrs are required.
    pub bootstrap_seed_peers: Vec<String>,
    /// Public relay/mediator candidates. Full `/p2p/<PeerId>` multiaddrs are required.
    pub relay_peers: Vec<String>,
}

impl Default for PublicBootstrapConfig {
    fn default() -> Self {
        Self {
            mode: PublicFallbackMode::Disabled,
            bootstrap_seed_peers: Vec::new(),
            relay_peers: Vec::new(),
        }
    }
}

impl PublicBootstrapConfig {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if self.mode.is_enabled()
            && self.bootstrap_seed_peers.is_empty()
            && self.relay_peers.is_empty()
        {
            return Err(config_error(
                "discovery.public_bootstrap mode is enabled but no public bootstrap or relay peers are configured",
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

fn config_error(reason: impl Into<String>) -> crate::common::error::NetError {
    crate::common::error::NetError::Config {
        path: "<config>".to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_only_requires_empty_owned_candidates() {
        let cfg = PublicBootstrapConfig {
            mode: PublicFallbackMode::FallbackOnly,
            bootstrap_seed_peers: vec!["/ip4/127.0.0.1/tcp/1/p2p/peer".to_string()],
            relay_peers: Vec::new(),
        };

        assert!(!cfg.bootstrap_decision(1).used);
        assert!(cfg.bootstrap_decision(0).used);
    }
}
