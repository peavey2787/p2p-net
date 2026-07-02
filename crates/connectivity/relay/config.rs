use crate::common::error::{config_error, NetError};
use super::schedule::RelaySchedule;
use super::state::RelayServiceHealth;
use std::num::NonZeroU32;
use std::time::Duration;

use libp2p::PeerId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayAccess {
    #[default]
    AllowAll,
    AllowList,
    DenyList,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayServiceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub access: RelayAccess,
    #[serde(default)]
    pub allow_peers: Vec<String>,
    #[serde(default)]
    pub deny_peers: Vec<String>,
    #[serde(default = "default_max_reservations")]
    pub max_reservations: usize,
    #[serde(default = "default_max_reservations_per_peer")]
    pub max_reservations_per_peer: usize,
    #[serde(default = "default_reservation_duration_secs")]
    pub reservation_duration_secs: u64,
    #[serde(default = "default_max_circuits")]
    pub max_circuits: usize,
    #[serde(default = "default_max_circuits_per_peer")]
    pub max_circuits_per_peer: usize,
    #[serde(default = "default_max_circuit_duration_secs")]
    pub max_circuit_duration_secs: u64,
    #[serde(default = "default_max_circuit_bytes")]
    pub max_circuit_bytes: u64,
    #[serde(default = "default_reservation_rate_per_peer_per_min")]
    pub reservation_rate_per_peer_per_min: u32,
    #[serde(default = "default_reservation_rate_per_ip_per_min")]
    pub reservation_rate_per_ip_per_min: u32,
    #[serde(default = "default_circuit_rate_per_peer_per_min")]
    pub circuit_rate_per_peer_per_min: u32,
    #[serde(default = "default_circuit_rate_per_ip_per_min")]
    pub circuit_rate_per_ip_per_min: u32,
    #[serde(default)]
    pub schedule: RelaySchedule,
}

impl Default for RelayServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            access: RelayAccess::default(),
            allow_peers: Vec::new(),
            deny_peers: Vec::new(),
            max_reservations: default_max_reservations(),
            max_reservations_per_peer: default_max_reservations_per_peer(),
            reservation_duration_secs: default_reservation_duration_secs(),
            max_circuits: default_max_circuits(),
            max_circuits_per_peer: default_max_circuits_per_peer(),
            max_circuit_duration_secs: default_max_circuit_duration_secs(),
            max_circuit_bytes: default_max_circuit_bytes(),
            reservation_rate_per_peer_per_min: default_reservation_rate_per_peer_per_min(),
            reservation_rate_per_ip_per_min: default_reservation_rate_per_ip_per_min(),
            circuit_rate_per_peer_per_min: default_circuit_rate_per_peer_per_min(),
            circuit_rate_per_ip_per_min: default_circuit_rate_per_ip_per_min(),
            schedule: RelaySchedule::default(),
        }
    }
}

impl RelayServiceConfig {
    pub fn validate(&self) -> Result<(), NetError> {
        validate_peer_ids("relay.allow_peers", &self.allow_peers)?;
        validate_peer_ids("relay.deny_peers", &self.deny_peers)?;
        self.schedule.validate()?;
        if self.max_reservations == 0 {
            return Err(config_error("relay.max_reservations must be at least 1"));
        }
        if self.max_reservations_per_peer == 0 {
            return Err(config_error(
                "relay.max_reservations_per_peer must be at least 1",
            ));
        }
        if self.max_reservations_per_peer > self.max_reservations {
            return Err(config_error(
                "relay.max_reservations_per_peer must be <= relay.max_reservations",
            ));
        }
        if self.max_circuits == 0 {
            return Err(config_error("relay.max_circuits must be at least 1"));
        }
        if self.max_circuits_per_peer == 0 {
            return Err(config_error(
                "relay.max_circuits_per_peer must be at least 1",
            ));
        }
        if self.max_circuits_per_peer > self.max_circuits {
            return Err(config_error(
                "relay.max_circuits_per_peer must be <= relay.max_circuits",
            ));
        }
        if self.reservation_duration_secs == 0 {
            return Err(config_error(
                "relay.reservation_duration_secs must be at least 1",
            ));
        }
        if self.max_circuit_duration_secs == 0 {
            return Err(config_error(
                "relay.max_circuit_duration_secs must be at least 1",
            ));
        }
        if self.max_circuit_bytes == 0 {
            return Err(config_error("relay.max_circuit_bytes must be at least 1"));
        }
        Ok(())
    }

    pub fn is_active_now(&self) -> bool {
        self.enabled && self.schedule.is_open_now_utc()
    }

    pub fn health_now(&self) -> RelayServiceHealth {
        if !self.enabled {
            RelayServiceHealth::Disabled
        } else if !self.schedule.is_open_now_utc() {
            RelayServiceHealth::ClosedBySchedule
        } else {
            RelayServiceHealth::Enabled
        }
    }

    pub fn denied_peer_ids(&self) -> Vec<PeerId> {
        parse_peer_ids(&self.deny_peers)
    }

    pub fn allowed_peer_ids(&self) -> Vec<PeerId> {
        parse_peer_ids(&self.allow_peers)
    }

    pub fn allows_peer(&self, peer: &PeerId) -> bool {
        if contains_peer(&self.deny_peers, peer) {
            return false;
        }

        match self.access {
            RelayAccess::AllowAll | RelayAccess::DenyList => true,
            RelayAccess::AllowList => contains_peer(&self.allow_peers, peer),
        }
    }

    pub fn to_libp2p_config(&self) -> libp2p::relay::Config {
        let mut cfg = libp2p::relay::Config {
            max_reservations: self.max_reservations,
            max_reservations_per_peer: self.max_reservations_per_peer,
            reservation_duration: Duration::from_secs(self.reservation_duration_secs),
            max_circuits: self.max_circuits,
            max_circuits_per_peer: self.max_circuits_per_peer,
            max_circuit_duration: Duration::from_secs(
                self.max_circuit_duration_secs.min(u32::MAX as u64),
            ),
            max_circuit_bytes: self.max_circuit_bytes,
            ..Default::default()
        };

        cfg.reservation_rate_limiters.clear();
        cfg.circuit_src_rate_limiters.clear();

        let minute = Duration::from_secs(60);
        if let Some(limit) = NonZeroU32::new(self.reservation_rate_per_peer_per_min) {
            cfg = cfg.reservation_rate_per_peer(limit, minute);
        }
        if let Some(limit) = NonZeroU32::new(self.reservation_rate_per_ip_per_min) {
            cfg = cfg.reservation_rate_per_ip(limit, minute);
        }
        if let Some(limit) = NonZeroU32::new(self.circuit_rate_per_peer_per_min) {
            cfg = cfg.circuit_src_per_peer(limit, minute);
        }
        if let Some(limit) = NonZeroU32::new(self.circuit_rate_per_ip_per_min) {
            cfg = cfg.circuit_src_per_ip(limit, minute);
        }

        cfg
    }
}

fn validate_peer_ids(field: &str, values: &[String]) -> Result<(), NetError> {
    for raw in values {
        raw.parse::<PeerId>().map_err(|err| {
            config_error(format!("{field} contains invalid peer id `{raw}`: {err}"))
        })?;
    }
    Ok(())
}

fn parse_peer_ids(values: &[String]) -> Vec<PeerId> {
    values
        .iter()
        .filter_map(|raw| raw.parse::<PeerId>().ok())
        .collect()
}

fn contains_peer(values: &[String], peer: &PeerId) -> bool {
    values
        .iter()
        .any(|raw| raw.parse::<PeerId>().ok().as_ref() == Some(peer))
}

fn default_max_reservations() -> usize {
    64
}
fn default_max_reservations_per_peer() -> usize {
    1
}
fn default_reservation_duration_secs() -> u64 {
    60 * 60
}
fn default_max_circuits() -> usize {
    128
}
fn default_max_circuits_per_peer() -> usize {
    4
}
fn default_max_circuit_duration_secs() -> u64 {
    2 * 60
}
fn default_max_circuit_bytes() -> u64 {
    100 * 1024 * 1024
}
fn default_reservation_rate_per_peer_per_min() -> u32 {
    4
}
fn default_reservation_rate_per_ip_per_min() -> u32 {
    16
}
fn default_circuit_rate_per_peer_per_min() -> u32 {
    16
}
fn default_circuit_rate_per_ip_per_min() -> u32 {
    64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_service_is_disabled_by_default() {
        let cfg = RelayServiceConfig::default();
        assert!(!cfg.enabled);
        assert!(!cfg.is_active_now());
    }

    #[test]
    fn deny_list_wins_over_allow_list() {
        let peer = PeerId::random();
        let cfg = RelayServiceConfig {
            enabled: true,
            access: RelayAccess::AllowList,
            allow_peers: vec![peer.to_string()],
            deny_peers: vec![peer.to_string()],
            ..RelayServiceConfig::default()
        };
        assert!(!cfg.allows_peer(&peer));
    }
}
