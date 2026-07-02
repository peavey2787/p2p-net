//! DCUtR upgrade policy and fallback bookkeeping.
//!
//! libp2p performs the low-level hole-punch protocol. This module keeps the
//! product-level policy explicit so callers can see when direct connections are
//! preferred, when relayed fallback is retained, and how aggressively retries
//! may be attempted.

use crate::common::error::config_error;
use serde::{Deserialize, Serialize};

/// Operator policy for Direct Connection Upgrade through Relay (DCUtR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DcutrPolicy {
    /// Master switch for DCUtR. When disabled, the behaviour is not installed
    /// and relayed connections remain relay-only.
    pub enabled: bool,
    /// Treat newly established relayed connections as eligible for DCUtR.
    /// rust-libp2p's DCUtR behaviour performs the protocol-level negotiation;
    /// p2p-net uses this switch for explicit policy, observability, and future
    /// retry scheduling.
    pub attempt_after_relay_connection: bool,
    /// Keep the relay circuit as a safe fallback when a direct upgrade fails or
    /// cannot be attempted. Lite/mobile profiles should normally leave this on.
    pub keep_relay_fallback: bool,
    /// Minimum seconds between repeat upgrade attempts to the same peer.
    pub retry_interval_secs: u64,
    /// Maximum DCUtR attempts per peer before relying on relay fallback.
    pub max_attempts_per_peer: u32,
}

impl Default for DcutrPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            attempt_after_relay_connection: true,
            keep_relay_fallback: true,
            retry_interval_secs: 60,
            max_attempts_per_peer: 3,
        }
    }
}

impl DcutrPolicy {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if !self.enabled {
            return Ok(());
        }
        if self.retry_interval_secs == 0 {
            return Err(config_error(
                "dcutr.retry_interval_secs must be at least 1 when DCUtR is enabled",
            ));
        }
        if self.max_attempts_per_peer == 0 {
            return Err(config_error(
                "dcutr.max_attempts_per_peer must be at least 1 when DCUtR is enabled",
            ));
        }
        if self.attempt_after_relay_connection && !self.keep_relay_fallback {
            return Err(config_error(
                "dcutr.keep_relay_fallback must be true when attempting upgrades after relayed connections",
            ));
        }
        Ok(())
    }
}
