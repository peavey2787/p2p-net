use std::collections::{BTreeSet, HashMap, HashSet};
use std::num::NonZeroU32;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libp2p::autonat;
use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayServiceHealth {
    #[default]
    Disabled,
    Enabled,
    ClosedBySchedule,
    AtCapacity,
    RateLimited,
    Error,
}

impl RelayServiceHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::ClosedBySchedule => "closed_by_schedule",
            Self::AtCapacity => "at_capacity",
            Self::RateLimited => "rate_limited",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RelayState {
    pub behind_restrictive_nat: bool,
    pub reservation_attempted: bool,
    pub server_enabled: bool,
    pub health: RelayServiceHealth,
    pub accepted_reservations: usize,
    pub active_circuits: usize,
    pub denied_reservations: usize,
    pub denied_circuits: usize,
    pub rate_limited_events: usize,
    pub at_capacity_events: usize,
    pub server_errors: usize,
    pub relay_client_reservations: HashSet<PeerId>,
    pub relay_client_reservation_attempts: usize,
    pub relay_client_reservation_failures: usize,
    pub relay_discovery_selected_relays: BTreeSet<String>,
    pub relay_discovery_candidate_count: usize,
    pub relay_discovery_configured_candidates: usize,
    pub relay_discovery_cached_candidates: usize,
    pub relay_discovery_rendezvous_candidates: usize,
    pub relay_discovery_public_candidates: usize,
    pub relay_discovery_ignored_candidates: usize,
    pub relay_discovery_failures: usize,
    pub relay_discovery_replacements: usize,
    pub relayed_listen_addrs: BTreeSet<String>,
    pub relay_bytes_forwarded: u64,
    pub dcutr_enabled: bool,
    pub dcutr_attempts: usize,
    pub dcutr_successes: usize,
    pub dcutr_failures: usize,
    pub dcutr_relay_fallbacks: usize,
    pub dcutr_upgrade_eligible_connections: usize,
    pub dcutr_retry_suppressed: usize,
    pub dcutr_attempts_by_peer: HashMap<PeerId, u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayReservationPlan {
    pub attempted: usize,
    pub listen_addrs: Vec<Multiaddr>,
    pub errors: Vec<String>,
}

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
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
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

        // Replace libp2p's default relay rate limiters with user-configured values.
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelaySchedule {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub windows: Vec<RelayWindow>,
}

impl RelaySchedule {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        for window in &self.windows {
            window.validate()?;
        }
        Ok(())
    }

    pub fn is_open_now_utc(&self) -> bool {
        if !self.enabled {
            return true;
        }
        let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return false;
        };
        let days_since_epoch = elapsed.as_secs() / 86_400;
        let minute_of_day = ((elapsed.as_secs() % 86_400) / 60) as u16;
        let day = ((days_since_epoch + 4) % 7) as u8; // 0=sun, 1=mon, ..., 6=sat.
        self.is_open_at_utc(day, minute_of_day)
    }

    /// Deterministic UTC schedule check. `day` uses 0=sun, 1=mon, ..., 6=sat.
    pub fn is_open_at_utc(&self, day: u8, minute_of_day: u16) -> bool {
        if !self.enabled {
            return true;
        }
        if self.windows.is_empty() || day > 6 || minute_of_day >= 1_440 {
            return false;
        }

        let previous_day = if day == 0 { 6 } else { day - 1 };
        self.windows
            .iter()
            .any(|window| window.matches(day, previous_day, minute_of_day))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayWindow {
    /// UTC days: "sun", "mon", "tue", "wed", "thu", "fri", "sat", or "all".
    pub days: Vec<String>,
    /// UTC start time in HH:MM, e.g. "18:00".
    pub start: String,
    /// UTC end time in HH:MM, e.g. "23:00". End is exclusive.
    pub end: String,
}

impl RelayWindow {
    fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if self.days.is_empty() {
            return Err(config_error(
                "relay.schedule.windows entries must include at least one day",
            ));
        }
        for day in &self.days {
            if !is_valid_day(day) {
                return Err(config_error(format!(
                    "relay.schedule.windows contains invalid day `{day}`"
                )));
            }
        }
        if parse_hhmm(&self.start).is_none() {
            return Err(config_error(format!(
                "relay.schedule.windows contains invalid start time `{}`",
                self.start
            )));
        }
        if parse_hhmm(&self.end).is_none() {
            return Err(config_error(format!(
                "relay.schedule.windows contains invalid end time `{}`",
                self.end
            )));
        }
        Ok(())
    }

    fn matches(&self, day: u8, previous_day: u8, minute_of_day: u16) -> bool {
        let Some(start) = parse_hhmm(&self.start) else {
            return false;
        };
        let Some(end) = parse_hhmm(&self.end) else {
            return false;
        };

        if start == end {
            return self.matches_day(day);
        }

        if start < end {
            return self.matches_day(day) && minute_of_day >= start && minute_of_day < end;
        }

        (self.matches_day(day) && minute_of_day >= start)
            || (self.matches_day(previous_day) && minute_of_day < end)
    }

    fn matches_day(&self, day: u8) -> bool {
        self.days
            .iter()
            .any(|raw| match raw.trim().to_ascii_lowercase().as_str() {
                "all" | "*" => true,
                "sun" | "sunday" => day == 0,
                "mon" | "monday" => day == 1,
                "tue" | "tues" | "tuesday" => day == 2,
                "wed" | "wednesday" => day == 3,
                "thu" | "thur" | "thurs" | "thursday" => day == 4,
                "fri" | "friday" => day == 5,
                "sat" | "saturday" => day == 6,
                _ => false,
            })
    }
}

pub fn update_nat_state(state: &mut RelayState, event: &autonat::Event) {
    if let autonat::Event::StatusChanged { old: _, new } = event {
        state.behind_restrictive_nat = matches!(
            new,
            autonat::NatStatus::Private | autonat::NatStatus::Unknown
        );
    }
}

/// Convert a configured relay peer address into the relayed listen address
/// that makes rust-libp2p request a Circuit Relay v2 reservation.
///
/// Example:
/// `/ip4/127.0.0.1/tcp/4001/p2p/<relay>` ->
/// `/ip4/127.0.0.1/tcp/4001/p2p/<relay>/p2p-circuit`
pub fn relay_reservation_addr(relay_addr: &Multiaddr) -> Option<Multiaddr> {
    if !has_p2p_peer_id(relay_addr) || is_p2p_circuit_addr(relay_addr) {
        return None;
    }

    Some(relay_addr.clone().with(Protocol::P2pCircuit))
}

/// Return true for any address that contains `/p2p-circuit`.
pub fn is_p2p_circuit_addr(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
}

/// Extract the relay peer ID from a relay or relayed address.
///
/// For `/ip4/.../p2p/<relay>/p2p-circuit/p2p/<target>`, this returns `<relay>`.
pub fn relay_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    for protocol in addr.iter() {
        match protocol {
            Protocol::P2p(peer) => return Some(peer),
            Protocol::P2pCircuit => return None,
            _ => {}
        }
    }
    None
}

fn has_p2p_peer_id(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::P2p(_)))
}

fn validate_peer_ids(field: &str, values: &[String]) -> Result<(), crate::common::error::NetError> {
    for raw in values {
        raw.parse::<PeerId>().map_err(|err| {
            config_error(format!("{field} contains invalid peer id `{raw}`: {err}"))
        })?;
    }
    Ok(())
}

fn config_error(reason: impl Into<String>) -> crate::common::error::NetError {
    crate::common::error::NetError::Config {
        path: "<config>".to_string(),
        reason: reason.into(),
    }
}

fn is_valid_day(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "all"
            | "*"
            | "sun"
            | "sunday"
            | "mon"
            | "monday"
            | "tue"
            | "tues"
            | "tuesday"
            | "wed"
            | "wednesday"
            | "thu"
            | "thur"
            | "thurs"
            | "thursday"
            | "fri"
            | "friday"
            | "sat"
            | "saturday"
    )
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

fn parse_hhmm(value: &str) -> Option<u16> {
    let mut parts = value.split(':');
    let hour = parts.next()?.parse::<u16>().ok()?;
    let minute = parts.next()?.parse::<u16>().ok()?;
    if parts.next().is_some() || hour >= 24 || minute >= 60 {
        return None;
    }
    Some(hour * 60 + minute)
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

pub fn classify_relay_denial(status_debug: &str) -> RelayServiceHealth {
    let lower = status_debug.to_ascii_lowercase();
    if lower.contains("rate") || lower.contains("thrott") {
        RelayServiceHealth::RateLimited
    } else if lower.contains("resource")
        || lower.contains("limit")
        || lower.contains("capacity")
        || lower.contains("too")
        || lower.contains("no reservation")
    {
        RelayServiceHealth::AtCapacity
    } else {
        RelayServiceHealth::Error
    }
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

    #[test]
    fn parses_hhmm() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
    }

    #[test]
    fn builds_relay_reservation_address() {
        let relay_peer = PeerId::random();
        let relay_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay_peer}")
            .parse()
            .unwrap();
        let reservation = relay_reservation_addr(&relay_addr).unwrap();
        assert_eq!(
            reservation.to_string(),
            format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay_peer}/p2p-circuit")
        );
        assert_eq!(relay_peer_id(&reservation), Some(relay_peer));
        assert!(is_p2p_circuit_addr(&reservation));
    }

    #[test]
    fn reservation_address_rejects_non_relay_addr() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        assert!(relay_reservation_addr(&addr).is_none());
    }
}
