//! Bounded periodic connection and relay maintenance for the node runtime.

use std::collections::BTreeSet;
use std::time::Duration;

use libp2p::{PeerId, Swarm};

use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::DhtProviderState;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::relay::{RelayReservationPlan, RelayState};
use crate::connectivity::relay_discovery::RelaySelectionPlan;
use crate::stack::MeshBehaviour;

use super::dial::{auto_dial_peer_from_book, AutoDialOutcome, AutoDialStats};
use super::{NodeConfig, ResolvedNodeConfig};

const UNVERIFIED_RELAY_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn initial_relay_state(
    cfg: &NodeConfig,
    resolved_config: &ResolvedNodeConfig,
    relay_reservation_plan: &RelayReservationPlan,
    relay_selection_plan: &RelaySelectionPlan,
) -> RelayState {
    // Build from `Default` and mutate the public initialization fields rather
    // than using struct-update syntax. `RelayState` deliberately keeps its
    // internal DCUtR eviction queue private so callers cannot desynchronize
    // it from the public retry-history maps.
    let mut state = RelayState::default();
    state.server_enabled = cfg.relay.is_active_now();
    state.health = cfg.relay.health_now();
    state.relay_client_reservation_attempts = relay_reservation_plan.attempted;
    state.relay_client_reservation_failures = relay_reservation_plan.errors.len();
    state.dcutr_enabled = resolved_config.dcutr_enabled;
    state.relay_discovery_selected_relays = relay_selection_plan
        .selected_strings()
        .into_iter()
        .collect::<BTreeSet<_>>();
    state.relay_discovery_candidate_count = relay_selection_plan.total_candidates();
    state.relay_discovery_configured_candidates = relay_selection_plan.configured_candidates;
    state.relay_discovery_cached_candidates = relay_selection_plan.cached_candidates;
    state.relay_discovery_rendezvous_candidates = relay_selection_plan.rendezvous_candidates;
    state.relay_discovery_public_candidates = relay_selection_plan.public_candidates;
    state.relay_discovery_ignored_candidates = relay_selection_plan.ignored_candidates;
    state.relay_discovery_failures = relay_selection_plan
        .errors
        .len()
        .saturating_add(relay_reservation_plan.errors.len());
    state.relay_discovery_replacements = 0;
    // Keep requested relay listeners out of advertised reachability until
    // the relay transport confirms them with `NewListenAddr`.
    state.relayed_listen_addrs = BTreeSet::new();
    state
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn maintain_application_connections(
    cfg: &NodeConfig,
    swarm: &mut Swarm<MeshBehaviour>,
    local_peer: PeerId,
    peer_book: &PeerBook,
    pending_connections: &mut PendingConnectionPlans,
    dht_state: &mut DhtProviderState,
    auto_dial_stats: &mut AutoDialStats,
) -> Vec<String> {
    if !cfg.discovery.public_bootstrap.auto_connect_discovered_peers {
        return Vec::new();
    }

    let candidates = peer_book
        .reconnect_candidates()
        .filter(|peer| !auto_dial_stats.is_suppressed(peer))
        // Keep every heartbeat bounded even if a hostile discovery source
        // fills the peer book with namespace-compatible identities.
        .take(8)
        .collect::<Vec<_>>();
    let mut pulses = Vec::new();

    for peer in candidates {
        if !dht_state.should_auto_connect_provider_result(&peer) {
            continue;
        }
        let outcome = auto_dial_peer_from_book(
            peer,
            local_peer,
            true,
            swarm,
            peer_book,
            pending_connections,
            &cfg.dcutr,
        );
        auto_dial_stats.record_outcome(&peer, &outcome);
        match &outcome {
            AutoDialOutcome::DialStarted(_) | AutoDialOutcome::AddressResolutionStarted(_) => {
                dht_state.mark_auto_connect_attempted(peer);
            }
            AutoDialOutcome::DialFailed(_) => {
                dht_state.mark_auto_connect_attempted(peer);
                dht_state.mark_auto_connect_failed(&peer);
            }
            AutoDialOutcome::AwaitingAddress => {
                dht_state.mark_auto_connect_waiting_for_addrs(peer);
            }
            _ => {}
        }
        if outcome.should_pulse() {
            pulses.push(format!(
                "application peer reconnect {}",
                outcome.describe(&peer)
            ));
        }
    }

    pulses
}

pub(crate) fn close_expired_unverified_relayed(
    swarm: &mut Swarm<MeshBehaviour>,
    relay_state: &mut RelayState,
) -> Option<String> {
    let expired = relay_state
        .unverified_relayed_peers
        .iter()
        .filter_map(|(peer, first_seen)| {
            (first_seen.elapsed() >= UNVERIFIED_RELAY_TIMEOUT).then_some(*peer)
        })
        .collect::<Vec<_>>();
    if expired.is_empty() {
        return None;
    }

    for peer in &expired {
        relay_state.unverified_relayed_peers.remove(peer);
        let _ = swarm.disconnect_peer_id(*peer);
    }

    Some(format!(
        "closed {} unverified relayed peer(s) after {}s verification timeout",
        expired.len(),
        UNVERIFIED_RELAY_TIMEOUT.as_secs()
    ))
}
