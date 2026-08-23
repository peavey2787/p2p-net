use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

use libp2p::{autonat, Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

const MAX_TRACKED_DCUTR_PEERS: usize = 1_024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Currently active inbound reservations. Decremented when a reservation closes.
    pub accepted_reservations: usize,
    /// Cumulative non-renewal reservation acceptances since node start.
    pub accepted_reservations_total: usize,
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
    pub unverified_relayed_peers: HashMap<PeerId, Instant>,
    pub dcutr_enabled: bool,
    pub dcutr_attempts: usize,
    pub dcutr_successes: usize,
    pub dcutr_failures: usize,
    pub dcutr_relay_fallbacks: usize,
    pub dcutr_upgrade_eligible_connections: usize,
    pub dcutr_retry_suppressed: usize,
    pub dcutr_attempts_by_peer: HashMap<PeerId, u32>,
    pub dcutr_last_attempt_by_peer: HashMap<PeerId, Instant>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayReservationPlan {
    pub attempted: usize,
    pub listen_addrs: Vec<Multiaddr>,
    pub errors: Vec<String>,
}

impl RelayState {
    pub(crate) fn record_reservation_accepted(&mut self, renewed: bool) {
        if renewed {
            return;
        }
        self.accepted_reservations = self.accepted_reservations.saturating_add(1);
        self.accepted_reservations_total = self.accepted_reservations_total.saturating_add(1);
    }

    pub(crate) fn record_reservation_closed(&mut self) {
        self.accepted_reservations = self.accepted_reservations.saturating_sub(1);
    }

    /// Bound node-level DCUtR retry history; the behaviour has its own bound too.
    ///
    /// This intentionally does not keep a second ordering queue. The retry maps
    /// are historical bookkeeping only, so evicting any old identity at the
    /// backstop is sufficient and avoids a second structure that can accumulate
    /// stale entries or make `RelayState` harder for downstream callers to build.
    pub(crate) fn track_dcutr_peer(&mut self, peer: PeerId) {
        if self.dcutr_attempts_by_peer.contains_key(&peer) {
            return;
        }
        while self.dcutr_attempts_by_peer.len() >= MAX_TRACKED_DCUTR_PEERS {
            let Some(evicted) = self.dcutr_attempts_by_peer.keys().next().cloned() else {
                break;
            };
            self.dcutr_attempts_by_peer.remove(&evicted);
            self.dcutr_last_attempt_by_peer.remove(&evicted);
        }
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
mod tests;
