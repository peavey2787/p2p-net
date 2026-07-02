//! First-class DCUtR mediator policy.
//!
//! A mediator is a reachable node that intentionally runs Circuit Relay v2 so
//! lite/mobile peers can establish relayed connectivity and then attempt DCUtR
//! hole punching. The central resolver maps this policy onto relay-server
//! behaviour while keeping mediator intent visible in config, snapshots,
//! metrics, and docs.

use crate::common::error::config_error_at;
use serde::{Deserialize, Serialize};

use crate::connectivity::relay::{RelayAccess, RelayServiceConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MediatorConfig {
    /// Intentionally run this node as a DCUtR mediator. This enables the relay
    /// server capability through the central resolver.
    pub enabled: bool,
    /// Advertise this node as suitable for peers that need relayed connectivity
    /// before attempting DCUtR. Relay discovery and observability use this
    /// policy bit to distinguish intentional mediators from generic relays.
    pub advertise_for_dcutr: bool,
    /// Allow relay reservations. If false, validation rejects mediator startup
    /// because rust-libp2p's stock relay behaviour does not expose separate
    /// runtime switches for reservations and circuits.
    pub allow_reservations: bool,
    /// Allow relayed circuits. If false, validation rejects mediator startup
    /// because accepting reservations without circuits would mislead lite peers.
    pub allow_circuits: bool,
    /// Require peers to be authenticated/trusted before mediation. The current
    /// implementation maps this to the relay allow-list, so at least one allowed
    /// peer must be configured when this is true.
    pub require_authenticated_peers: bool,
    /// Coarse capacity cap for the number of peers this mediator is willing to
    /// serve. Mapped onto reservation/circuit caps when lower than relay limits.
    pub max_mediated_peers: usize,
    /// Mediator-level reservation cap. Mapped onto `relay.max_reservations`.
    pub max_reservations: usize,
    /// Mediator-level circuit cap. Mapped onto `relay.max_circuits`.
    pub max_circuits: usize,
}

impl Default for MediatorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            advertise_for_dcutr: true,
            allow_reservations: true,
            allow_circuits: true,
            require_authenticated_peers: false,
            max_mediated_peers: default_max_mediated_peers(),
            max_reservations: default_max_reservations(),
            max_circuits: default_max_circuits(),
        }
    }
}

impl MediatorConfig {
    pub fn validate(&self, relay: &RelayServiceConfig) -> Result<(), crate::common::error::NetError> {
        if !self.enabled {
            return Ok(());
        }
        if !self.allow_reservations {
            return Err(config_error_at(
                "<mediator>",
                "mediator.allow_reservations must be true while mediator.enabled is true",
            ));
        }
        if !self.allow_circuits {
            return Err(config_error_at(
                "<mediator>",
                "mediator.allow_circuits must be true while mediator.enabled is true",
            ));
        }
        if self.max_mediated_peers == 0 {
            return Err(config_error_at(
                "<mediator>",
                "mediator.max_mediated_peers must be at least 1",
            ));
        }
        if self.max_reservations == 0 {
            return Err(config_error_at(
                "<mediator>",
                "mediator.max_reservations must be at least 1",
            ));
        }
        if self.max_circuits == 0 {
            return Err(config_error_at(
                "<mediator>",
                "mediator.max_circuits must be at least 1",
            ));
        }
        if self.require_authenticated_peers {
            if !matches!(relay.access, RelayAccess::AllowList) {
                return Err(config_error_at(
                    "<mediator>",
                    "mediator.require_authenticated_peers requires relay.access = allow_list",
                ));
            }
            if relay.allow_peers.is_empty() {
                return Err(config_error_at(
                    "<mediator>",
                    "mediator.require_authenticated_peers requires relay.allow_peers to contain at least one trusted peer",
                ));
            }
        }
        Ok(())
    }

    pub fn is_active_now(&self, relay: &RelayServiceConfig) -> bool {
        self.enabled && relay.is_active_now()
    }

    /// Map mediator intent onto the existing Circuit Relay v2 service config.
    /// The central resolver owns the final behaviour policy; this method only
    /// projects mediator capacity settings onto relay-service limits.
    pub fn apply_to_relay(&self, relay: &mut RelayServiceConfig) {
        if !self.enabled {
            return;
        }
        relay.enabled = true;
        relay.max_reservations = relay.max_reservations.min(self.max_reservations);
        relay.max_circuits = relay.max_circuits.min(self.max_circuits);
        relay.max_reservations_per_peer = relay
            .max_reservations_per_peer
            .min(self.max_mediated_peers)
            .max(1);
        relay.max_circuits_per_peer = relay
            .max_circuits_per_peer
            .min(self.max_mediated_peers)
            .max(1);
    }
}

fn default_max_mediated_peers() -> usize {
    64
}

fn default_max_reservations() -> usize {
    64
}

fn default_max_circuits() -> usize {
    128
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mediator_is_disabled_by_default() {
        let mediator = MediatorConfig::default();
        let relay = RelayServiceConfig::default();
        assert!(!mediator.enabled);
        assert!(!mediator.is_active_now(&relay));
        mediator.validate(&relay).expect("disabled mediator validates");
    }

    #[test]
    fn authenticated_mediator_requires_allow_list() {
        let mediator = MediatorConfig {
            enabled: true,
            require_authenticated_peers: true,
            ..MediatorConfig::default()
        };
        let relay = RelayServiceConfig {
            access: RelayAccess::AllowList,
            ..RelayServiceConfig::default()
        };
        assert!(mediator.validate(&relay).is_err());
    }

    #[test]
    fn mediator_policy_enables_and_caps_relay_service() {
        let mediator = MediatorConfig {
            enabled: true,
            max_mediated_peers: 2,
            max_reservations: 8,
            max_circuits: 16,
            ..MediatorConfig::default()
        };
        let mut relay = RelayServiceConfig::default();
        mediator.apply_to_relay(&mut relay);
        assert!(relay.enabled);
        assert_eq!(relay.max_reservations, 8);
        assert_eq!(relay.max_circuits, 16);
        assert_eq!(relay.max_reservations_per_peer, 1);
        assert_eq!(relay.max_circuits_per_peer, 2);
    }
}
