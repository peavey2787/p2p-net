use std::collections::{BTreeSet, HashMap, HashSet};

use libp2p::{autonat, Multiaddr, PeerId};
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
    pub relay_client_attempted_peers: HashSet<PeerId>,
    pub relay_client_reservation_attempts: usize,
    pub relay_client_reservation_failures: usize,
    pub pending_relay_listen_addrs: HashMap<PeerId, BTreeSet<String>>,
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

pub fn update_nat_state(state: &mut RelayState, event: &autonat::Event) {
    if let autonat::Event::StatusChanged { old: _, new } = event {
        state.behind_restrictive_nat = matches!(
            new,
            autonat::NatStatus::Private | autonat::NatStatus::Unknown
        );
    }
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
