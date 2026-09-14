use super::*;

#[test]
fn auto_connect_tracking_queues_do_not_accumulate_stale_entries() {
    let mut state = DhtProviderState::default();
    let peer = PeerId::random();

    for _ in 0..10_000 {
        assert!(state.mark_auto_connect_waiting_for_addrs(peer));
        assert!(state.mark_auto_connect_attempted(peer));
        assert!(state.mark_auto_connect_failed(&peer));
    }

    assert!(state.auto_connect_attempted_peers.is_empty());
    assert!(state.auto_connect_waiting_for_addrs.is_empty());
    assert!(state.auto_connect_attempted_order.is_empty());
    assert!(state.auto_connect_waiting_order.is_empty());
}
