use super::*;

#[test]
fn dcutr_retry_history_is_bounded() {
    let mut state = RelayState::default();
    for _ in 0..MAX_TRACKED_DCUTR_PEERS + 32 {
        let peer = PeerId::random();
        state.track_dcutr_peer(peer);
        state.dcutr_attempts_by_peer.insert(peer, 1);
        state
            .dcutr_last_attempt_by_peer
            .insert(peer, Instant::now());
    }

    assert_eq!(state.dcutr_attempts_by_peer.len(), MAX_TRACKED_DCUTR_PEERS);
    assert_eq!(
        state.dcutr_last_attempt_by_peer.len(),
        MAX_TRACKED_DCUTR_PEERS
    );
}

#[test]
fn reservation_acceptance_total_is_monotonic_while_active_count_tracks_lifetime() {
    let mut state = RelayState::default();

    state.record_reservation_accepted(false);
    assert_eq!(state.accepted_reservations, 1);
    assert_eq!(state.accepted_reservations_total, 1);

    state.record_reservation_accepted(true);
    assert_eq!(state.accepted_reservations, 1);
    assert_eq!(state.accepted_reservations_total, 1);

    state.record_reservation_closed();
    assert_eq!(state.accepted_reservations, 0);
    assert_eq!(state.accepted_reservations_total, 1);

    state.record_reservation_closed();
    assert_eq!(state.accepted_reservations, 0);
    assert_eq!(state.accepted_reservations_total, 1);
}
