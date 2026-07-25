//! Bounded periodic connection and relay maintenance for the node runtime.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use libp2p::{PeerId, Swarm};
use tokio::sync::Mutex;

use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::DhtProviderState;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::relay::{RelayReservationPlan, RelayState};
use crate::connectivity::relay_discovery::RelaySelectionPlan;
use crate::stack::MeshBehaviour;

use super::dial::{auto_dial_peer_from_book, AutoDialOutcome, AutoDialStats};
use super::{NodeConfig, NodeSnapshot, ResolvedNodeConfig};

const UNVERIFIED_RELAY_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn initial_relay_state(
    cfg: &NodeConfig,
    resolved_config: &ResolvedNodeConfig,
    relay_reservation_plan: &RelayReservationPlan,
    relay_selection_plan: &RelaySelectionPlan,
) -> RelayState {
    RelayState {
        server_enabled: cfg.relay.is_active_now(),
        health: cfg.relay.health_now(),
        relay_client_reservation_attempts: relay_reservation_plan.attempted,
        relay_client_reservation_failures: relay_reservation_plan.errors.len(),
        dcutr_enabled: resolved_config.dcutr_enabled,
        relay_discovery_selected_relays: relay_selection_plan
            .selected_strings()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        relay_discovery_candidate_count: relay_selection_plan.total_candidates(),
        relay_discovery_configured_candidates: relay_selection_plan.configured_candidates,
        relay_discovery_cached_candidates: relay_selection_plan.cached_candidates,
        relay_discovery_rendezvous_candidates: relay_selection_plan.rendezvous_candidates,
        relay_discovery_public_candidates: relay_selection_plan.public_candidates,
        relay_discovery_ignored_candidates: relay_selection_plan.ignored_candidates,
        relay_discovery_failures: relay_selection_plan
            .errors
            .len()
            .saturating_add(relay_reservation_plan.errors.len()),
        relay_discovery_replacements: 0,
        // Keep requested relay listeners out of advertised reachability until
        // the relay transport confirms them with `NewListenAddr`.
        relayed_listen_addrs: BTreeSet::new(),
        ..RelayState::default()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn maintain_application_connections(
    cfg: &NodeConfig,
    swarm: &mut Swarm<MeshBehaviour>,
    local_peer: PeerId,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    peer_book: &PeerBook,
    pending_connections: &mut PendingConnectionPlans,
    dht_state: &mut DhtProviderState,
    auto_dial_stats: &mut AutoDialStats,
) {
    if !cfg.discovery.public_bootstrap.auto_connect_discovered_peers {
        return;
    }

    let candidates = peer_book
        .records()
        .filter(|record| !record.connected && !record.namespaces.is_empty())
        .map(|record| record.peer_id)
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

    if pulses.is_empty() {
        return;
    }
    let mut guard = snapshot.lock().await;
    guard.auto_connect_dial_attempts = auto_dial_stats.dial_attempts;
    guard.auto_connect_dial_failures = auto_dial_stats.dial_failures;
    guard.auto_connect_awaiting_address_peers = auto_dial_stats.awaiting_address_count();
    guard.connection_plan_pending_peers = pending_connections.pending_count();
    for pulse in pulses {
        super::push_pulse(&mut guard.pulses, pulse);
    }
}

pub(crate) async fn close_expired_unverified_relayed(
    swarm: &mut Swarm<MeshBehaviour>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
) {
    let expired = relay_state
        .unverified_relayed_peers
        .iter()
        .filter_map(|(peer, first_seen)| {
            (first_seen.elapsed() >= UNVERIFIED_RELAY_TIMEOUT).then_some(*peer)
        })
        .collect::<Vec<_>>();
    if expired.is_empty() {
        return;
    }

    for peer in &expired {
        relay_state.unverified_relayed_peers.remove(peer);
        let _ = swarm.disconnect_peer_id(*peer);
    }

    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    super::push_pulse(
        &mut guard.pulses,
        format!(
            "closed {} unverified relayed peer(s) after {}s verification timeout",
            expired.len(),
            UNVERIFIED_RELAY_TIMEOUT.as_secs()
        ),
    );
}
