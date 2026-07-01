//! Connection planning for `connect_peer(...)`.
//!
//! The public primitive still accepts one concrete multiaddr. Internally, the
//! planner expands that request with peer-book addresses, orders direct QUIC
//! paths before other direct paths, keeps relayed paths as fallback candidates,
//! and records whether a successful relayed path should be considered DCUtR
//! eligible by the connection event policy.

use std::collections::{BTreeSet, HashMap, VecDeque};

use libp2p::{Multiaddr, PeerId};

use crate::connectivity::dcutr::DcutrPolicy;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::relay::is_p2p_circuit_addr;

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
        if remaining.is_empty() {
            self.pending.remove(&peer);
        } else {
            self.pending.insert(peer, remaining);
        }
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
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[must_use]
pub fn build_connection_plan(
    requested_addr: Multiaddr,
    peer_book: &PeerBook,
    dcutr_policy: &DcutrPolicy,
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
        .any(|protocol| matches!(protocol.to_string().as_str(), "quic" | "quic-v1"))
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
mod tests {
    use super::*;
    use crate::api::PeerSource;

    fn addr(peer: PeerId, transport: &str, port: u16) -> Multiaddr {
        format!("/ip4/127.0.0.1/{transport}/{port}/p2p/{peer}")
            .parse()
            .expect("valid addr")
    }

    #[test]
    fn planner_prefers_direct_quic_then_direct_then_relay() {
        let peer = PeerId::random();
        let tcp = addr(peer, "tcp", 4001);
        let quic = format!("/ip4/127.0.0.1/udp/4002/quic-v1/p2p/{peer}")
            .parse::<Multiaddr>()
            .expect("valid quic addr");
        let relay = format!("/ip4/127.0.0.1/tcp/4003/p2p/{peer}/p2p-circuit/p2p/{peer}")
            .parse::<Multiaddr>()
            .expect("valid relay addr");
        let mut book = PeerBook::default();
        book.record_addr(peer, quic.clone(), PeerSource::PeerCache);
        book.record_addr(peer, relay, PeerSource::RelayDiscovery);

        let plan = build_connection_plan(tcp, &book, &DcutrPolicy::default());

        assert_eq!(plan.attempts[0].addr, quic);
        assert_eq!(plan.attempts[0].kind, ConnectionAttemptKind::DirectQuic);
        assert_eq!(plan.attempts[1].kind, ConnectionAttemptKind::Direct);
        assert_eq!(plan.attempts[2].kind, ConnectionAttemptKind::Relay);
        assert!(plan.attempt_dcutr_after_relay);
        assert!(plan.keep_relay_fallback);
    }

    #[test]
    fn planner_uses_relay_first_for_relay_preferred_peer() {
        let peer = PeerId::random();
        let tcp = addr(peer, "tcp", 4001);
        let relay = format!("/ip4/127.0.0.1/tcp/4003/p2p/{peer}/p2p-circuit/p2p/{peer}")
            .parse::<Multiaddr>()
            .expect("valid relay addr");
        let mut book = PeerBook::default();
        book.record_addr(peer, relay.clone(), PeerSource::RelayDiscovery);
        book.record_relay_preferred(peer, true);

        let plan = build_connection_plan(tcp, &book, &DcutrPolicy::default());

        assert!(plan.relay_preferred);
        assert_eq!(plan.attempts[0].addr, relay);
        assert_eq!(plan.attempts[0].kind, ConnectionAttemptKind::Relay);
    }

    #[test]
    fn pending_plans_return_remaining_attempts_after_failure() {
        let peer = PeerId::random();
        let tcp = addr(peer, "tcp", 4001);
        let quic = format!("/ip4/127.0.0.1/udp/4002/quic-v1/p2p/{peer}")
            .parse::<Multiaddr>()
            .expect("valid quic addr");
        let mut book = PeerBook::default();
        book.record_addr(peer, tcp.clone(), PeerSource::PeerCache);
        let plan = build_connection_plan(quic.clone(), &book, &DcutrPolicy::default());
        let first = plan.first_attempt().expect("first attempt").clone();
        let mut pending = PendingConnectionPlans::default();

        pending.track_remaining(&plan, &first);
        let fallback = pending.next_after_failure(&peer).expect("fallback attempt");

        assert_eq!(fallback.addr, tcp);
        assert_eq!(pending.pending_count(), 0);
    }
}
