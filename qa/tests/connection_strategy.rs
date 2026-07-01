use libp2p::{Multiaddr, PeerId};
use p2p_net::{
    build_connection_plan, ConnectionAttemptKind, DcutrPolicy, PeerBook, PeerSource,
};

#[test]
fn connection_plan_prefers_quic_direct_then_tcp_then_relay() {
    let peer = PeerId::random();
    let tcp = tcp_addr(peer, 4101);
    let quic = quic_addr(peer, 4102);
    let relay = relay_addr(peer, 4103);
    let mut book = PeerBook::default();
    book.record_addr(peer, tcp.clone(), PeerSource::PeerCache);
    book.record_addr(peer, quic.clone(), PeerSource::Rendezvous);

    let plan = build_connection_plan(relay.clone(), &book, &DcutrPolicy::default());

    assert_eq!(plan.target_peer, Some(peer));
    assert_eq!(plan.attempts.len(), 3);
    assert_eq!(plan.attempts[0].addr, quic);
    assert_eq!(plan.attempts[0].kind, ConnectionAttemptKind::DirectQuic);
    assert_eq!(plan.attempts[1].addr, tcp);
    assert_eq!(plan.attempts[1].kind, ConnectionAttemptKind::Direct);
    assert_eq!(plan.attempts[2].addr, relay);
    assert_eq!(plan.attempts[2].kind, ConnectionAttemptKind::Relay);
    assert!(plan.attempt_dcutr_after_relay);
    assert!(plan.keep_relay_fallback);
}

#[test]
fn relay_preferred_peer_uses_relay_before_direct_candidates() {
    let peer = PeerId::random();
    let tcp = tcp_addr(peer, 4201);
    let relay = relay_addr(peer, 4202);
    let mut book = PeerBook::default();
    book.record_addr(peer, relay.clone(), PeerSource::RelayDiscovery);
    book.record_relay_preferred(peer, true);

    let plan = build_connection_plan(tcp, &book, &DcutrPolicy::default());

    assert!(plan.relay_preferred);
    assert_eq!(plan.attempts[0].addr, relay);
    assert_eq!(plan.attempts[0].kind, ConnectionAttemptKind::Relay);
}

#[test]
fn dcutr_flags_follow_operator_policy() {
    let peer = PeerId::random();
    let addr = relay_addr(peer, 4301);
    let policy = DcutrPolicy {
        enabled: false,
        attempt_after_relay_connection: false,
        keep_relay_fallback: true,
        ..DcutrPolicy::default()
    };

    let plan = build_connection_plan(addr, &PeerBook::default(), &policy);

    assert!(!plan.attempt_dcutr_after_relay);
    assert!(plan.keep_relay_fallback);
}

fn tcp_addr(peer: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer}")
        .parse()
        .expect("valid tcp addr")
}

fn quic_addr(peer: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/udp/{port}/quic-v1/p2p/{peer}")
        .parse()
        .expect("valid quic addr")
}

fn relay_addr(peer: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer}/p2p-circuit/p2p/{peer}")
        .parse()
        .expect("valid relay addr")
}
