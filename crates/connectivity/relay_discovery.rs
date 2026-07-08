//! Relay discovery and selection policy for lite/mobile nodes.
//!
//! Relay discovery stays DRY and deterministic: configured relays, cached
//! relay candidates, and rendezvous-discovered relay candidates all flow through
//! one selector before reservation attempts are made.

use crate::common::error::config_error;
use std::collections::BTreeSet;

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

use super::addr::{has_reachable_transport, has_unspecified_ip};
use super::relay::{is_p2p_circuit_addr, relay_peer_id};

/// Operator policy for finding relay/mediator candidates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayDiscoveryPolicy {
    /// Master switch for automatic relay discovery/selection. When disabled,
    /// only explicitly configured `relay_peers` are considered.
    pub enabled: bool,
    /// Include explicit `relay_peers` from `NodeConfig`. Operators use these to
    /// pin trusted relays for controlled deployments.
    pub use_configured_relays: bool,
    /// Include healthy identity-bound addresses from the peer cache as relay
    /// candidates. Failed/expired cache entries are already filtered by the peer cache.
    pub use_cached_relays: bool,
    /// Include relay-like peers learned through rendezvous discovery. This is
    /// intentionally a policy switch because rendezvous records do not prove a
    /// peer is willing to relay; failed candidates are counted and replaced.
    pub use_rendezvous_relays: bool,
    /// Inspect Identify protocols on public-DHT peers and request reservations
    /// from peers that advertise Circuit Relay v2 hop service.
    pub use_dht_relays: bool,
    /// Minimum desired relay reservations for NATed/lite nodes.
    pub min_reservations: usize,
    /// Maximum simultaneous relay candidates/reservation attempts.
    pub max_reservations: usize,
    /// Prefer configured relays before cached/rendezvous candidates.
    pub prefer_configured_relays: bool,
    /// Replace failed or unhealthy relays with the next known candidate.
    pub replace_failed_relays: bool,
}

impl Default for RelayDiscoveryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            use_configured_relays: true,
            use_cached_relays: true,
            use_rendezvous_relays: true,
            use_dht_relays: true,
            min_reservations: 3,
            max_reservations: 8,
            prefer_configured_relays: true,
            replace_failed_relays: true,
        }
    }
}

impl RelayDiscoveryPolicy {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if !self.enabled {
            return Ok(());
        }
        if self.max_reservations == 0 {
            return Err(config_error(
                "discovery.relay_discovery.max_reservations must be at least 1",
            ));
        }
        if self.min_reservations > self.max_reservations {
            return Err(config_error(
                "discovery.relay_discovery.min_reservations must be <= max_reservations",
            ));
        }
        if !self.use_configured_relays
            && !self.use_cached_relays
            && !self.use_rendezvous_relays
            && !self.use_dht_relays
        {
            return Err(config_error(
                "discovery.relay_discovery must enable at least one relay source",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayCandidateSource {
    Configured,
    Cached,
    Rendezvous,
    PublicFallback,
}

impl RelayCandidateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Cached => "cached",
            Self::Rendezvous => "rendezvous",
            Self::PublicFallback => "public_fallback",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayCandidate {
    pub peer_id: String,
    pub addr: String,
    pub source: RelayCandidateSource,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelaySelectionPlan {
    pub enabled: bool,
    pub selected_addrs: Vec<Multiaddr>,
    pub configured_candidates: usize,
    pub cached_candidates: usize,
    pub rendezvous_candidates: usize,
    pub public_candidates: usize,
    pub ignored_candidates: usize,
    pub min_reservations: usize,
    pub max_reservations: usize,
    pub errors: Vec<String>,
}

impl RelaySelectionPlan {
    pub fn selected_strings(&self) -> Vec<String> {
        self.selected_addrs
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    pub fn total_candidates(&self) -> usize {
        self.configured_candidates
            .saturating_add(self.cached_candidates)
            .saturating_add(self.rendezvous_candidates)
            .saturating_add(self.public_candidates)
    }
}

/// Select a deterministic, deduplicated relay candidate set for startup.
pub fn select_startup_relays(
    policy: &RelayDiscoveryPolicy,
    configured_relays: Vec<Multiaddr>,
    cached_relays: Vec<Multiaddr>,
    rendezvous_relays: Vec<Multiaddr>,
    public_relays: Vec<Multiaddr>,
) -> RelaySelectionPlan {
    let mut plan = RelaySelectionPlan {
        enabled: policy.enabled,
        min_reservations: policy.min_reservations,
        max_reservations: policy.max_reservations,
        ..RelaySelectionPlan::default()
    };

    if !policy.enabled {
        plan.selected_addrs = valid_relay_candidates(configured_relays)
            .into_iter()
            .map(|candidate| candidate.addr)
            .take(policy.max_reservations.max(1))
            .collect();
        plan.configured_candidates = plan.selected_addrs.len();
        plan.ignored_candidates = cached_relays
            .len()
            .saturating_add(rendezvous_relays.len())
            .saturating_add(public_relays.len());
        return plan;
    }

    let mut seen = BTreeSet::<String>::new();
    let mut ordered = Vec::<RelayCandidateAddr>::new();

    let sources: Vec<(RelayCandidateSource, Vec<Multiaddr>, bool)> =
        if policy.prefer_configured_relays {
            vec![
                (
                    RelayCandidateSource::Configured,
                    configured_relays,
                    policy.use_configured_relays,
                ),
                (
                    RelayCandidateSource::Cached,
                    cached_relays,
                    policy.use_cached_relays,
                ),
                (
                    RelayCandidateSource::Rendezvous,
                    rendezvous_relays,
                    policy.use_rendezvous_relays,
                ),
                (RelayCandidateSource::PublicFallback, public_relays, true),
            ]
        } else {
            vec![
                (
                    RelayCandidateSource::Cached,
                    cached_relays,
                    policy.use_cached_relays,
                ),
                (
                    RelayCandidateSource::Rendezvous,
                    rendezvous_relays,
                    policy.use_rendezvous_relays,
                ),
                (
                    RelayCandidateSource::Configured,
                    configured_relays,
                    policy.use_configured_relays,
                ),
                (RelayCandidateSource::PublicFallback, public_relays, true),
            ]
        };

    for (source, addrs, enabled) in sources {
        if !enabled {
            plan.ignored_candidates = plan.ignored_candidates.saturating_add(addrs.len());
            continue;
        }

        for addr in addrs {
            let Some(candidate) = relay_candidate_addr(addr, source) else {
                plan.ignored_candidates = plan.ignored_candidates.saturating_add(1);
                continue;
            };

            if !seen.insert(candidate.peer_id.to_string()) {
                plan.ignored_candidates = plan.ignored_candidates.saturating_add(1);
                continue;
            }

            match source {
                RelayCandidateSource::Configured => {
                    plan.configured_candidates = plan.configured_candidates.saturating_add(1)
                }
                RelayCandidateSource::Cached => {
                    plan.cached_candidates = plan.cached_candidates.saturating_add(1)
                }
                RelayCandidateSource::Rendezvous => {
                    plan.rendezvous_candidates = plan.rendezvous_candidates.saturating_add(1)
                }
                RelayCandidateSource::PublicFallback => {
                    plan.public_candidates = plan.public_candidates.saturating_add(1)
                }
            }

            ordered.push(candidate);
        }
    }

    plan.selected_addrs = ordered
        .into_iter()
        .map(|candidate| candidate.addr)
        .take(policy.max_reservations)
        .collect();

    if plan.selected_addrs.len() < policy.min_reservations {
        plan.errors.push(format!(
            "relay discovery selected {} candidates but min_reservations={} is requested",
            plan.selected_addrs.len(),
            policy.min_reservations
        ));
    }

    plan
}

/// Filter addresses to relay candidates while preserving original order.
pub fn valid_relay_candidates(addrs: Vec<Multiaddr>) -> Vec<RelayCandidateAddr> {
    addrs
        .into_iter()
        .filter_map(|addr| relay_candidate_addr(addr, RelayCandidateSource::Configured))
        .collect()
}

/// Convert a multiaddr into a relay candidate if it is identity-bound,
/// reachable, and not already a relayed `/p2p-circuit` address.
pub fn relay_candidate_addr(
    addr: Multiaddr,
    source: RelayCandidateSource,
) -> Option<RelayCandidateAddr> {
    if is_p2p_circuit_addr(&addr) || !has_reachable_transport(&addr) || has_unspecified_ip(&addr) {
        return None;
    }
    let peer_id = relay_peer_id(&addr)?;
    Some(RelayCandidateAddr {
        peer_id,
        addr,
        source,
    })
}

/// Score native relay transports in preferred dial order.
///
/// TLS WebSocket remains supported because the native WebSocket transport
/// handles it. WebTransport and WebRTC-direct are rejected until native
/// transports for those multiaddr protocols are installed.
pub(crate) fn supported_relay_addr_score(addr: &Multiaddr) -> Option<u8> {
    if addr.iter().any(|protocol| {
        matches!(
            protocol,
            Protocol::WebTransport | Protocol::WebRTCDirect | Protocol::P2pWebRtcDirect
        )
    }) {
        return None;
    }
    if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Quic | Protocol::QuicV1))
    {
        Some(0)
    } else if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Tcp(_)))
    {
        Some(1)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayCandidateAddr {
    pub peer_id: PeerId,
    pub addr: Multiaddr,
    pub source: RelayCandidateSource,
}

impl RelayCandidateAddr {
    pub fn to_public_candidate(&self) -> RelayCandidate {
        RelayCandidate {
            peer_id: self.peer_id.to_string(),
            addr: self.addr.to_string(),
            source: self.source,
        }
    }
}
