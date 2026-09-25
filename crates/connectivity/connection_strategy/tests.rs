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
fn peer_book_planner_uses_known_addresses_without_manual_addr() {
    let peer = PeerId::random();
    let quic = format!("/ip4/127.0.0.1/udp/4002/quic-v1/p2p/{peer}")
        .parse::<Multiaddr>()
        .expect("valid quic addr");
    let mut book = PeerBook::default();
    book.record_addr(peer, quic.clone(), PeerSource::DhtProvider);

    let plan = build_peer_book_connection_plan(peer, &book, &DcutrPolicy::default());

    assert_eq!(plan.target_peer, Some(peer));
    assert_eq!(plan.attempts.len(), 1);
    assert_eq!(plan.attempts[0].addr, quic);
    assert_eq!(plan.attempts[0].kind, ConnectionAttemptKind::DirectQuic);
}

#[test]
fn pending_plans_track_inflight_peer_dedupe() {
    let peer = PeerId::random();
    let tcp = addr(peer, "tcp", 4001);
    let plan = build_connection_plan(tcp, &PeerBook::default(), &DcutrPolicy::default());
    let first = plan.first_attempt().expect("first attempt").clone();
    let mut pending = PendingConnectionPlans::default();

    pending.track_remaining(&plan, &first);

    assert!(pending.is_pending(&peer));
    assert_eq!(pending.next_after_failure(&peer), None);
    assert!(!pending.is_pending(&peer));
}

#[test]
fn browser_policy_filters_native_only_transports_before_ordering() {
    let peer = PeerId::random();
    let quic = format!("/dns4/example.test/udp/443/quic-v1/p2p/{peer}")
        .parse::<Multiaddr>()
        .unwrap();
    let webrtc = format!("/dns4/example.test/udp/4001/webrtc-direct/certhash/uEiAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/p2p/{peer}")
        .parse::<Multiaddr>();
    let wss = format!("/dns4/example.test/tcp/443/wss/p2p/{peer}")
        .parse::<Multiaddr>()
        .unwrap();
    let relay = format!("/dns4/relay.example/tcp/443/wss/p2p/{peer}/p2p-circuit/p2p/{peer}")
        .parse::<Multiaddr>()
        .unwrap();
    let mut book = PeerBook::default();
    book.record_addr(peer, quic, PeerSource::PeerCache);
    if let Ok(webrtc) = webrtc {
        book.record_addr(peer, webrtc, PeerSource::PeerCache);
    }
    book.record_addr(peer, relay.clone(), PeerSource::RelayDiscovery);

    let plan = build_connection_plan_with_capabilities(
        wss.clone(),
        &book,
        &DcutrPolicy::default(),
        &TransportCapabilities::browser(),
    );

    assert!(plan.attempts.iter().all(|attempt| {
        let raw = attempt.addr.to_string();
        !raw.contains("/quic-v1") || raw.contains("/webtransport")
    }));
    assert!(plan.attempts.iter().any(|attempt| attempt.addr == wss));
    assert!(plan.attempts.iter().any(|attempt| attempt.addr == relay));
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
