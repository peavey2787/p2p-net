//! Connection planning for `connect_peer(...)`.
//!
//! The public primitive still accepts one concrete multiaddr. Internally, the
//! planner expands that request with peer-book addresses, orders direct QUIC
//! paths before other direct paths, keeps relayed paths as fallback candidates,
//! and records whether a successful relayed path should be considered DCUtR
//! eligible by the connection event policy.

use std::collections::{BTreeSet, HashMap, VecDeque};

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

use crate::connectivity::dcutr::DcutrPolicy;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::relay::is_p2p_circuit_addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransportCapabilities {
    pub tcp: bool,
    pub quic: bool,
    pub webrtc_direct: bool,
    pub secure_websocket: bool,
    pub webtransport: bool,
    pub circuit_relay: bool,
}

impl TransportCapabilities {
    pub const fn native() -> Self {
        Self {
            tcp: true,
            quic: true,
            webrtc_direct: true,
            secure_websocket: true,
            webtransport: false,
            circuit_relay: true,
        }
    }

    pub const fn browser() -> Self {
        Self {
            tcp: false,
            quic: false,
            webrtc_direct: true,
            secure_websocket: true,
            // WebTransport is compiled in for an incremental follow-up but is
            // not advertised/dial-selected until the swarm builder enables it.
            webtransport: false,
            circuit_relay: true,
        }
    }

    #[must_use]
    pub fn allows(&self, addr: &Multiaddr) -> bool {
        let raw = addr.to_string();
        let transport = raw.split("/p2p-circuit").next().unwrap_or(raw.as_str());
        let is_relay = raw.contains("/p2p-circuit");
        if is_relay && !self.circuit_relay {
            return false;
        }
        if transport.contains("/webrtc-direct") {
            return self.webrtc_direct;
        }
        if transport.contains("/webtransport") {
            return self.webtransport;
        }
        if transport.contains("/wss") || transport.contains("/tls/ws") {
            return self.secure_websocket;
        }
        if transport.contains("/quic") {
            return self.quic;
        }
        if transport.contains("/tcp/") {
            return self.tcp;
        }
        // Native remains open to memory/custom transports; browser policy is
        // deliberately an allow-list of transports browsers can actually dial.
        *self == Self::native()
    }
}

impl Default for TransportCapabilities {
    fn default() -> Self {
        if cfg!(target_arch = "wasm32") {
            Self::browser()
        } else {
            Self::native()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionAttemptKind {
    DirectQuic,
    Direct,
    Relay,
}

impl ConnectionAttemptKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DirectQuic => "direct_quic",
            Self::Direct => "direct",
            Self::Relay => "relay",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionAttempt {
    pub addr: Multiaddr,
    pub kind: ConnectionAttemptKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionPlan {
    pub target_peer: Option<PeerId>,
    pub attempts: Vec<ConnectionAttempt>,
    pub relay_preferred: bool,
    pub attempt_dcutr_after_relay: bool,
    pub keep_relay_fallback: bool,
}

impl ConnectionPlan {
    #[must_use]
    pub fn first_attempt(&self) -> Option<&ConnectionAttempt> {
        self.attempts.first()
    }

    #[must_use]
    pub fn describe(&self) -> String {
        let kinds = self
            .attempts
            .iter()
            .map(|attempt| attempt.kind.as_str())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "target_peer={:?} attempts={} relay_preferred={} dcutr_after_relay={} keep_relay_fallback={} order=[{}]",
            self.target_peer,
            self.attempts.len(),
            self.relay_preferred,
            self.attempt_dcutr_after_relay,
            self.keep_relay_fallback,
            kinds
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct PendingConnectionPlans {
    pending: HashMap<PeerId, VecDeque<ConnectionAttempt>>,
}

impl PendingConnectionPlans {
    pub fn track_remaining(&mut self, plan: &ConnectionPlan, attempted: &ConnectionAttempt) {
        let Some(peer) = plan.target_peer.as_ref().cloned() else {
            return;
        };
        let mut seen_attempted = false;
        let remaining = plan
            .attempts
            .iter()
            .filter_map(|candidate| {
                if !seen_attempted {
                    if candidate == attempted {
                        seen_attempted = true;
                    }
                    None
                } else {
                    Some(candidate.clone())
                }
            })
            .collect::<VecDeque<_>>();
        self.pending.insert(peer, remaining);
    }

    pub fn next_after_failure(&mut self, peer: &PeerId) -> Option<ConnectionAttempt> {
        let next = self.pending.get_mut(peer).and_then(VecDeque::pop_front);
        let empty = self
            .pending
            .get(peer)
            .map(VecDeque::is_empty)
            .unwrap_or(false);
        if empty {
            self.pending.remove(peer);
        }
        next
    }

    pub fn complete(&mut self, peer: &PeerId) {
        self.pending.remove(peer);
    }

    #[must_use]
    pub fn is_pending(&self, peer: &PeerId) -> bool {
        self.pending.contains_key(peer)
    }

    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[must_use]
pub fn build_peer_book_connection_plan(
    peer_id: PeerId,
    peer_book: &PeerBook,
    dcutr_policy: &DcutrPolicy,
) -> ConnectionPlan {
    build_peer_book_connection_plan_with_capabilities(
        peer_id,
        peer_book,
        dcutr_policy,
        &TransportCapabilities::default(),
    )
}

#[must_use]
pub fn build_peer_book_connection_plan_with_capabilities(
    peer_id: PeerId,
    peer_book: &PeerBook,
    dcutr_policy: &DcutrPolicy,
    capabilities: &TransportCapabilities,
) -> ConnectionPlan {
    let relay_preferred = peer_book
        .record(&peer_id)
        .map(|record| record.relay_preferred)
        .unwrap_or(false);
    let mut candidate_strings = BTreeSet::new();
    let mut candidates = Vec::new();

    if let Some(peer) = peer_book.record(&peer_id) {
        for addr in &peer.addresses {
            if let Ok(addr) = addr.parse::<Multiaddr>() {
                push_candidate(&mut candidates, &mut candidate_strings, addr);
            }
        }
    }

    candidates.retain(|addr| capabilities.allows(addr));
    let attempts = ordered_attempts(candidates, relay_preferred);
    ConnectionPlan {
        target_peer: Some(peer_id),
        attempts,
        relay_preferred,
        attempt_dcutr_after_relay: dcutr_policy.enabled
            && dcutr_policy.attempt_after_relay_connection,
        keep_relay_fallback: dcutr_policy.keep_relay_fallback,
    }
}

#[must_use]
pub fn build_connection_plan(
    requested_addr: Multiaddr,
    peer_book: &PeerBook,
    dcutr_policy: &DcutrPolicy,
) -> ConnectionPlan {
    build_connection_plan_with_capabilities(
        requested_addr,
        peer_book,
        dcutr_policy,
        &TransportCapabilities::default(),
    )
}

#[must_use]
pub fn build_connection_plan_with_capabilities(
    requested_addr: Multiaddr,
    peer_book: &PeerBook,
    dcutr_policy: &DcutrPolicy,
    capabilities: &TransportCapabilities,
) -> ConnectionPlan {
    let target_peer = extract_p2p_peer_id(&requested_addr);
    let relay_preferred = target_peer
        .as_ref()
        .and_then(|peer| peer_book.record(peer))
        .map(|record| record.relay_preferred)
        .unwrap_or(false);

    let mut candidate_strings = BTreeSet::new();
    let mut candidates = Vec::new();
    push_candidate(&mut candidates, &mut candidate_strings, requested_addr);

    if let Some(peer) = target_peer.as_ref().and_then(|peer| peer_book.record(peer)) {
        for addr in &peer.addresses {
            if let Ok(addr) = addr.parse::<Multiaddr>() {
                push_candidate(&mut candidates, &mut candidate_strings, addr);
            }
        }
    }

    candidates.retain(|addr| capabilities.allows(addr));
    let attempts = ordered_attempts(candidates, relay_preferred);
    ConnectionPlan {
        target_peer,
        attempts,
        relay_preferred,
        attempt_dcutr_after_relay: dcutr_policy.enabled
            && dcutr_policy.attempt_after_relay_connection,
        keep_relay_fallback: dcutr_policy.keep_relay_fallback,
    }
}

fn push_candidate(candidates: &mut Vec<Multiaddr>, seen: &mut BTreeSet<String>, addr: Multiaddr) {
    if seen.insert(addr.to_string()) {
        candidates.push(addr);
    }
}

fn ordered_attempts(candidates: Vec<Multiaddr>, relay_preferred: bool) -> Vec<ConnectionAttempt> {
    let mut direct_quic = Vec::new();
    let mut direct = Vec::new();
    let mut relay = Vec::new();

    for addr in candidates {
        let kind = classify_addr(&addr);
        let attempt = ConnectionAttempt { addr, kind };
        match kind {
            ConnectionAttemptKind::DirectQuic => direct_quic.push(attempt),
            ConnectionAttemptKind::Direct => direct.push(attempt),
            ConnectionAttemptKind::Relay => relay.push(attempt),
        }
    }

    let mut ordered = Vec::new();
    if relay_preferred {
        ordered.extend(relay);
        ordered.extend(direct_quic);
        ordered.extend(direct);
    } else {
        ordered.extend(direct_quic);
        ordered.extend(direct);
        ordered.extend(relay);
    }
    ordered
}

fn classify_addr(addr: &Multiaddr) -> ConnectionAttemptKind {
    if is_p2p_circuit_addr(addr) {
        return ConnectionAttemptKind::Relay;
    }
    if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Quic | Protocol::QuicV1))
    {
        return ConnectionAttemptKind::DirectQuic;
    }
    ConnectionAttemptKind::Direct
}

fn extract_p2p_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    let mut peer = None;
    for protocol in addr.iter() {
        if let libp2p::multiaddr::Protocol::P2p(id) = protocol {
            peer = Some(id);
        }
    }
    peer
}

#[cfg(test)]
mod tests;
