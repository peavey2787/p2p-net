use super::*;

#[test]
fn dcutr_retry_history_is_bounded() {
    let mut state = RelayState::default();
    for _ in 0..MAX_TRACKED_DCUTR_PEERS + 32 {
        let peer = PeerId::random();
        state.track_dcutr_peer(peer);
        state.dcutr_attempts_by_peer.insert(peer, 1);
        state.dcutr_last_attempt_by_peer.insert(peer, Instant::now());
    }

    assert_eq!(state.dcutr_attempts_by_peer.len(), MAX_TRACKED_DCUTR_PEERS);
    assert_eq!(state.dcutr_last_attempt_by_peer.len(), MAX_TRACKED_DCUTR_PEERS);
}
