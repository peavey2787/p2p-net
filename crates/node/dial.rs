use std::collections::{HashSet, VecDeque};

use libp2p::{PeerId, Swarm};

use crate::common::error::NetError;
use crate::connectivity::connection_strategy::{
    build_peer_book_connection_plan, ConnectionAttempt, ConnectionPlan, PendingConnectionPlans,
};
use crate::connectivity::dcutr::DcutrPolicy;
use crate::connectivity::peer_book::PeerBook;
use crate::stack::MeshBehaviour;

const MAX_AUTO_DIAL_AWAITING_PEERS: usize = 2048;

#[derive(Debug, Default)]
pub(crate) struct AutoDialStats {
    pub(crate) dial_attempts: usize,
    pub(crate) dial_failures: usize,
    awaiting_address_peers: HashSet<PeerId>,
    awaiting_address_order: VecDeque<PeerId>,
    suppressed_peers: HashSet<PeerId>,
    suppressed_order: VecDeque<PeerId>,
}

impl AutoDialStats {
    pub(crate) fn record_outcome(&mut self, peer: &PeerId, outcome: &AutoDialOutcome) {
        match outcome {
            AutoDialOutcome::DialStarted(_) => {
                self.dial_attempts = self.dial_attempts.saturating_add(1);
                self.remove_awaiting(peer);
            }
            AutoDialOutcome::DialFailed(_) => {
                self.dial_attempts = self.dial_attempts.saturating_add(1);
                self.dial_failures = self.dial_failures.saturating_add(1);
                self.remove_awaiting(peer);
            }
            AutoDialOutcome::AwaitingAddress => {
                self.mark_awaiting_address(*peer);
            }
            AutoDialOutcome::AddressResolutionStarted(_) => {
                self.dial_attempts = self.dial_attempts.saturating_add(1);
                self.mark_awaiting_address(*peer);
            }
            AutoDialOutcome::AlreadyConnected => {
                self.remove_awaiting(peer);
            }
            _ => {}
        }
    }

    pub(crate) fn clear_awaiting(&mut self, peer: &PeerId) {
        self.remove_awaiting(peer);
    }

    pub(crate) fn record_async_failure(&mut self, peer: &PeerId) {
        self.dial_failures = self.dial_failures.saturating_add(1);
        self.remove_awaiting(peer);
    }

    pub(crate) fn awaiting_address_count(&self) -> usize {
        self.awaiting_address_peers.len()
    }

    pub(crate) fn suppress_peer(&mut self, peer: PeerId) {
        if self.suppressed_peers.insert(peer) {
            self.suppressed_order.push_back(peer);
        }
        while self.suppressed_peers.len() > MAX_AUTO_DIAL_AWAITING_PEERS {
            let Some(evicted) = self.suppressed_order.pop_front() else {
                break;
            };
            self.suppressed_peers.remove(&evicted);
        }
    }

    pub(crate) fn allow_peer(&mut self, peer: &PeerId) {
        self.remove_suppressed(peer);
    }

    #[must_use]
    pub(crate) fn is_suppressed(&self, peer: &PeerId) -> bool {
        self.suppressed_peers.contains(peer)
    }

    fn remove_awaiting(&mut self, peer: &PeerId) -> bool {
        let removed = self.awaiting_address_peers.remove(peer);
        if removed {
            self.awaiting_address_order.retain(|queued| queued != peer);
        }
        removed
    }

    fn remove_suppressed(&mut self, peer: &PeerId) -> bool {
        let removed = self.suppressed_peers.remove(peer);
        if removed {
            self.suppressed_order.retain(|queued| queued != peer);
        }
        removed
    }

    fn mark_awaiting_address(&mut self, peer: PeerId) {
        if self.awaiting_address_peers.insert(peer) {
            self.awaiting_address_order.push_back(peer);
        }
        while self.awaiting_address_peers.len() > MAX_AUTO_DIAL_AWAITING_PEERS {
            let Some(evicted) = self.awaiting_address_order.pop_front() else {
                break;
            };
            self.awaiting_address_peers.remove(&evicted);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoDialOutcome {
    Disabled,
    LocalPeer,
    AlreadyConnected,
    AlreadyPending,
    AwaitingAddress,
    AddressResolutionStarted(String),
    DialStarted(String),
    DialFailed(String),
}

impl AutoDialOutcome {
    #[must_use]
    pub(crate) fn should_pulse(&self) -> bool {
        matches!(
            self,
            Self::AwaitingAddress
                | Self::AddressResolutionStarted(_)
                | Self::DialStarted(_)
                | Self::DialFailed(_)
        )
    }

    #[must_use]
    pub(crate) fn describe(&self, peer: &PeerId) -> String {
        match self {
            Self::Disabled => format!("auto-connect disabled peer={peer}"),
            Self::LocalPeer => format!("auto-connect skipped local peer={peer}"),
            Self::AlreadyConnected => format!("auto-connect skipped connected peer={peer}"),
            Self::AlreadyPending => format!("auto-connect skipped pending peer={peer}"),
            Self::AwaitingAddress => format!(
                "auto-connect awaiting dialable address peer={peer}; \
                 peer is known/discovered but not yet dialable"
            ),
            Self::AddressResolutionStarted(plan) => {
                format!("auto-connect address resolution started peer={peer} {plan}")
            }
            Self::DialStarted(plan) => format!("auto-connect dial started peer={peer} {plan}"),
            Self::DialFailed(reason) => {
                format!("auto-connect dial failed peer={peer} reason={reason}")
            }
        }
    }
}

pub(crate) fn auto_dial_peer_from_book(
    peer: PeerId,
    local_peer: PeerId,
    enabled: bool,
    swarm: &mut Swarm<MeshBehaviour>,
    peer_book: &PeerBook,
    pending_connections: &mut PendingConnectionPlans,
    dcutr_policy: &DcutrPolicy,
) -> AutoDialOutcome {
    if !enabled {
        return AutoDialOutcome::Disabled;
    }
    if peer == local_peer {
        return AutoDialOutcome::LocalPeer;
    }
    if swarm.connected_peers().any(|connected| connected == &peer) {
        return AutoDialOutcome::AlreadyConnected;
    }
    if pending_connections.is_pending(&peer) {
        return AutoDialOutcome::AlreadyPending;
    }

    let plan = build_peer_book_connection_plan(peer, peer_book, dcutr_policy);
    if plan.attempts.is_empty() {
        return AutoDialOutcome::AwaitingAddress;
    }

    let description = plan.describe();
    match dial_connection_plan(swarm, pending_connections, &plan) {
        Ok(()) => AutoDialOutcome::DialStarted(description),
        Err(err) => AutoDialOutcome::DialFailed(err.to_string()),
    }
}

pub(crate) fn auto_dial_dht_provider(
    peer: PeerId,
    local_peer: PeerId,
    enabled: bool,
    swarm: &mut Swarm<MeshBehaviour>,
    peer_book: &PeerBook,
    pending_connections: &mut PendingConnectionPlans,
    dcutr_policy: &DcutrPolicy,
) -> AutoDialOutcome {
    let outcome = auto_dial_peer_from_book(
        peer,
        local_peer,
        enabled,
        swarm,
        peer_book,
        pending_connections,
        dcutr_policy,
    );
    if !matches!(outcome, AutoDialOutcome::AwaitingAddress) {
        return outcome;
    }

    // GetProviders returns peer IDs, while the corresponding addresses can
    // still live only in Kademlia's routing table or active query state.
    // Dialing by PeerId lets NetworkBehaviour::handle_pending_outbound_connection
    // contribute those addresses instead of waiting for a later peer-book event
    // that may never arrive.
    match swarm.dial(peer) {
        Ok(()) => AutoDialOutcome::AddressResolutionStarted(
            "source=kademlia address_resolution=behaviour".to_string(),
        ),
        Err(_) => AutoDialOutcome::AwaitingAddress,
    }
}

pub(crate) fn dial_connection_plan(
    swarm: &mut Swarm<MeshBehaviour>,
    pending_connections: &mut PendingConnectionPlans,
    plan: &ConnectionPlan,
) -> Result<(), NetError> {
    let mut errors = Vec::new();
    for attempt in &plan.attempts {
        match dial_connection_attempt(swarm, attempt) {
            Ok(()) => {
                pending_connections.track_remaining(plan, attempt);
                return Ok(());
            }
            Err(err) => errors.push(err),
        }
    }

    Err(NetError::Dial {
        target: plan
            .target_peer
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<unknown>".to_string()),
        reason: if errors.is_empty() {
            format!("connection plan had no dial attempts: {}", plan.describe())
        } else {
            errors.join("; ")
        },
    })
}

fn dial_connection_attempt(
    swarm: &mut Swarm<MeshBehaviour>,
    attempt: &ConnectionAttempt,
) -> Result<(), String> {
    swarm.dial(attempt.addr.clone()).map(|_| ()).map_err(|err| {
        format!(
            "{} {} failed immediately: {}",
            attempt.kind.as_str(),
            attempt.addr,
            err
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_disconnect_suppression_is_bounded_and_reversible() {
        let mut stats = AutoDialStats::default();
        let retained = PeerId::random();
        stats.suppress_peer(retained);
        for _ in 0..MAX_AUTO_DIAL_AWAITING_PEERS {
            stats.suppress_peer(PeerId::random());
        }

        assert_eq!(stats.suppressed_peers.len(), MAX_AUTO_DIAL_AWAITING_PEERS);
        assert!(!stats.is_suppressed(&retained));

        let peer = PeerId::random();
        stats.suppress_peer(peer);
        assert!(stats.is_suppressed(&peer));
        stats.allow_peer(&peer);
        assert!(!stats.is_suppressed(&peer));
    }
}


#[cfg(test)]
mod memory_bound_tests {
    use super::*;

    #[test]
    fn auto_dial_order_queues_do_not_accumulate_removed_peers() {
        let mut stats = AutoDialStats::default();
        let peer = PeerId::random();

        for _ in 0..10_000 {
            stats.mark_awaiting_address(peer);
            stats.clear_awaiting(&peer);
            stats.suppress_peer(peer);
            stats.allow_peer(&peer);
        }

        assert!(stats.awaiting_address_peers.is_empty());
        assert!(stats.awaiting_address_order.is_empty());
        assert!(stats.suppressed_peers.is_empty());
        assert!(stats.suppressed_order.is_empty());
    }
}
