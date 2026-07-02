use libp2p::{PeerId, Swarm};

use crate::common::error::NetError;
use crate::connectivity::connection_strategy::{
    build_peer_book_connection_plan, ConnectionAttempt, ConnectionPlan, PendingConnectionPlans,
};
use crate::connectivity::dcutr::DcutrPolicy;
use crate::connectivity::peer_book::PeerBook;
use crate::stack::MeshBehaviour;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoDialOutcome {
    Disabled,
    LocalPeer,
    AlreadyConnected,
    AlreadyPending,
    AwaitingAddress,
    DialStarted(String),
    DialFailed(String),
}

impl AutoDialOutcome {
    #[must_use]
    pub(crate) fn should_pulse(&self) -> bool {
        matches!(
            self,
            Self::AwaitingAddress | Self::DialStarted(_) | Self::DialFailed(_)
        )
    }

    #[must_use]
    pub(crate) fn describe(&self, peer: &PeerId) -> String {
        match self {
            Self::Disabled => format!("auto-connect disabled peer={peer}"),
            Self::LocalPeer => format!("auto-connect skipped local peer={peer}"),
            Self::AlreadyConnected => format!("auto-connect skipped connected peer={peer}"),
            Self::AlreadyPending => format!("auto-connect skipped pending peer={peer}"),
            Self::AwaitingAddress => format!("auto-connect awaiting address peer={peer}"),
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
