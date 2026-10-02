//! Re-request relay reservations whose listener closed.
//!
//! The Circuit Relay v2 client reports neither a failed reservation (a stalled
//! request times out after 60 s) nor the loss of the relay connection as an
//! event of its own: both only close the `/p2p-circuit` listener. Without a
//! retry the node silently stays without a reservation on that relay, and
//! peers that can reach it only through the relay cannot reach it at all.

use std::collections::HashMap;
use std::time::Duration;

use libp2p::core::transport::ListenerId;
use libp2p::{Multiaddr, PeerId};
use web_time::Instant;

use super::relay_peer_id;

const BASE_DELAY: Duration = Duration::from_secs(5);
const MAX_DELAY: Duration = Duration::from_secs(120);

/// Reservation listeners and their pending re-requests.
#[derive(Debug, Clone, Default)]
pub struct ReservationRetries {
    listeners: HashMap<ListenerId, Multiaddr>,
    /// Consecutive closes per reservation address since the relay last
    /// accepted a reservation; drives the backoff.
    closes: HashMap<Multiaddr, u32>,
    due: Vec<(Instant, Multiaddr)>,
}

impl ReservationRetries {
    /// Remember the listener of a requested `/p2p-circuit` reservation.
    pub(crate) fn track(&mut self, listener: ListenerId, addr: Multiaddr) {
        self.listeners.insert(listener, addr);
    }

    /// A listener closed. For a reservation listener, schedules a re-request
    /// with exponential backoff and returns the address and delay.
    pub(crate) fn closed(
        &mut self,
        listener: ListenerId,
        now: Instant,
    ) -> Option<(Multiaddr, Duration)> {
        let addr = self.listeners.remove(&listener)?;
        let closes = self.closes.entry(addr.clone()).or_insert(0);
        let delay = BASE_DELAY
            .saturating_mul(1u32 << (*closes).min(5))
            .min(MAX_DELAY);
        *closes = closes.saturating_add(1);
        self.due.push((now + delay, addr.clone()));
        Some((addr, delay))
    }

    /// `relay` accepted a reservation: its addresses start over at the base
    /// delay the next time a listener closes.
    pub(crate) fn accepted(&mut self, relay: &PeerId) {
        self.closes
            .retain(|addr, _| relay_peer_id(addr).as_ref() != Some(relay));
    }

    /// Reservation addresses whose re-request is due.
    pub(crate) fn take_due(&mut self, now: Instant) -> Vec<Multiaddr> {
        let (due, later) = std::mem::take(&mut self.due)
            .into_iter()
            .partition::<Vec<_>, _>(|(at, _)| *at <= now);
        self.due = later;
        due.into_iter().map(|(_, addr)| addr).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reservation(relay: &PeerId) -> Multiaddr {
        format!("/ip4/10.0.0.1/tcp/4001/p2p/{relay}/p2p-circuit")
            .parse()
            .unwrap()
    }

    #[test]
    fn closed_reservation_listener_is_re_requested_with_backoff() {
        let relay = PeerId::random();
        let addr = reservation(&relay);
        let mut retries = ReservationRetries::default();
        let start = Instant::now();

        let mut delays = Vec::new();
        for _ in 0..8 {
            let listener = ListenerId::next();
            retries.track(listener, addr.clone());
            let (closed_addr, delay) = retries.closed(listener, start).expect("tracked");
            assert_eq!(closed_addr, addr);
            delays.push(delay.as_secs());
        }
        assert_eq!(delays, vec![5, 10, 20, 40, 80, 120, 120, 120]);

        assert!(retries.take_due(start).is_empty());
        assert_eq!(
            retries.take_due(start + Duration::from_secs(5)),
            vec![addr.clone()]
        );
        assert_eq!(retries.take_due(start + MAX_DELAY).len(), 7);
        assert!(retries.take_due(start + MAX_DELAY).is_empty());
    }

    #[test]
    fn other_listeners_are_ignored_and_acceptance_resets_backoff() {
        let relay = PeerId::random();
        let addr = reservation(&relay);
        let mut retries = ReservationRetries::default();
        let now = Instant::now();
        assert!(retries.closed(ListenerId::next(), now).is_none());

        for _ in 0..3 {
            let listener = ListenerId::next();
            retries.track(listener, addr.clone());
            retries.closed(listener, now);
        }
        retries.accepted(&PeerId::random());
        let listener = ListenerId::next();
        retries.track(listener, addr.clone());
        assert_eq!(retries.closed(listener, now).unwrap().1.as_secs(), 40);

        retries.accepted(&relay);
        let listener = ListenerId::next();
        retries.track(listener, addr.clone());
        assert_eq!(retries.closed(listener, now).unwrap().1, BASE_DELAY);
    }
}
