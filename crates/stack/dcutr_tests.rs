use super::*;

#[test]
fn verified_responder_remains_available_after_local_budget_is_spent() {
    let peer = PeerId::random();
    let relay = PeerId::random();
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 1);
    let candidate = "/ip4/8.8.4.4/tcp/4001".parse().unwrap();
    behaviour.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
        libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &candidate },
    ));
    behaviour.allow_peer(peer);
    assert!(behaviour.allow_relayed_upgrade(peer, true));
    assert!(!behaviour.allow_relayed_upgrade(peer, true));
    let addr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{peer}")
        .parse()
        .unwrap();
    let handler = behaviour
        .handle_established_outbound_connection(
            ConnectionId::new_unchecked(71),
            peer,
            &addr,
            Endpoint::Dialer,
            PortUse::Reuse,
        )
        .unwrap();
    assert!(
        handler.0.is_left(),
        "verified responders must still negotiate DCUtR"
    );
    assert_eq!(behaviour.attempts_by_peer[&peer], 1);
}

#[test]
fn late_verified_responder_does_not_spend_local_initiation_budget() {
    let peer = PeerId::random();
    let relay = PeerId::random();
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 1);
    let addr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{peer}")
        .parse()
        .unwrap();
    let handler = behaviour
        .handle_established_outbound_connection(
            ConnectionId::new_unchecked(72),
            peer,
            &addr,
            Endpoint::Dialer,
            PortUse::Reuse,
        )
        .unwrap();
    assert!(handler.0.is_right());
    let candidate = "/ip4/8.8.4.4/tcp/4001".parse().unwrap();
    behaviour.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
        libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &candidate },
    ));
    assert!(
        behaviour.pending_handlers.is_empty(),
        "unverified responders stay disabled"
    );
    behaviour.allow_peer(peer);
    assert_eq!(behaviour.pending_handlers.len(), 1);
    assert!(behaviour.attempts_by_peer.is_empty());
}

#[test]
fn late_verification_activates_existing_relay_handler_once() {
    let peer = PeerId::random();
    let relay = PeerId::random();
    let id = ConnectionId::new_unchecked(1);
    let addr = format!("/ip4/8.8.8.8/udp/4001/quic-v1/p2p/{relay}/p2p-circuit")
        .parse()
        .unwrap();
    let remote = format!("/p2p/{peer}").parse().unwrap();
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3);
    let candidate = "/ip4/8.8.4.4/tcp/4001".parse().unwrap();
    behaviour.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
        libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &candidate },
    ));
    let mut handler = behaviour
        .handle_established_inbound_connection(id, peer, &addr, &remote)
        .unwrap();
    assert!(handler.0.is_right());
    assert_eq!(behaviour.deferred.len(), 1);
    assert!(behaviour.attempts_by_peer.is_empty());

    behaviour.allow_peer(peer);
    assert!(behaviour.deferred.is_empty());
    assert_eq!(behaviour.attempts_by_peer[&peer], 1);
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    let Poll::Ready(ToSwarm::NotifyHandler {
        peer_id,
        handler: NotifyHandler::One(connection),
        event,
    }) = behaviour.poll(&mut cx)
    else {
        panic!("verification must activate the existing circuit");
    };
    assert_eq!(peer_id, peer);
    assert_eq!(connection, id);
    handler.on_behaviour_event(event);
    assert!(handler.0.is_left());
    assert!(matches!(
        handler.poll(&mut cx),
        Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest { .. })
    ));

    behaviour.allow_peer(peer);
    assert!(behaviour.pending_handlers.is_empty());
    assert_eq!(behaviour.attempts_by_peer[&peer], 1);
}

#[test]
fn verified_circuit_waits_for_a_public_candidate_without_spending_budget() {
    let peer = PeerId::random();
    let relay = PeerId::random();
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3).with_lan_candidates(false);
    behaviour.allow_peer(peer);
    let id = ConnectionId::new_unchecked(9);
    let addr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay}/p2p-circuit")
        .parse()
        .unwrap();
    let remote = format!("/p2p/{peer}").parse().unwrap();
    let handler = behaviour
        .handle_established_inbound_connection(id, peer, &addr, &remote)
        .unwrap();
    assert!(handler.0.is_right());
    assert!(behaviour.attempts_by_peer.is_empty());
    let private = "/ip4/192.168.0.53/tcp/4001".parse().unwrap();
    behaviour.on_swarm_event(FromSwarm::ExternalAddrConfirmed(
        libp2p::swarm::behaviour::ExternalAddrConfirmed { addr: &private },
    ));
    assert!(behaviour.pending_handlers.is_empty());
    let public = "/ip4/8.8.4.4/tcp/4001".parse().unwrap();
    // No preceding NewExternalAddrCandidate: Swarm can suppress that
    // notification for an address that has already been confirmed.
    behaviour.on_swarm_event(FromSwarm::ExternalAddrConfirmed(
        libp2p::swarm::behaviour::ExternalAddrConfirmed { addr: &public },
    ));
    assert!(behaviour.has_candidate);
    assert!(behaviour.deferred.is_empty());
    assert_eq!(behaviour.pending_handlers.len(), 1);
    assert_eq!(behaviour.attempts_by_peer[&peer], 1);
}

#[test]
fn wan_only_dcutr_does_not_offer_private_routes() {
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3).with_lan_candidates(false);
    for addr in [
        "/ip4/192.168.137.59/udp/4001/quic-v1",
        "/ip4/10.0.2.15/udp/4001/quic-v1",
        "/ip4/172.17.0.1/udp/4001/quic-v1",
        "/ip4/100.64.0.1/udp/4001/quic-v1",
        "/ip6/fd00::1/udp/4001/quic-v1",
    ] {
        assert!(
            !behaviour.accept_candidate(&addr.parse().unwrap()),
            "{addr}"
        );
    }
    assert!(behaviour.accept_candidate(&"/ip4/8.8.8.8/udp/4001/quic-v1".parse().unwrap()));
}

#[test]
fn dcutr_candidates_support_tcp_and_quic_without_protocol_suffixes() {
    let quic: Multiaddr = "/ip4/192.168.1.2/udp/4001/quic-v1".parse().unwrap();
    let tcp: Multiaddr = "/ip4/192.168.1.2/tcp/4001".parse().unwrap();
    let loopback: Multiaddr = "/ip4/127.0.0.1/udp/4001/quic-v1".parse().unwrap();

    assert!(is_dcutr_candidate(&quic));
    assert!(is_dcutr_candidate(&tcp));
    assert!(!is_dcutr_candidate(&loopback));
    for addr in [
        "/ip4/8.8.8.8/tcp/4001/ws",
        "/ip4/8.8.8.8/udp/4001/quic-v1/webtransport",
        "/ip4/8.8.8.8/tcp/0",
    ] {
        assert!(!is_dcutr_candidate(&addr.parse().unwrap()));
    }
}

#[test]
fn fresh_nat_mappings_remain_eligible_after_initial_candidates_fill() {
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 3).with_lan_candidates(false);
    // Memory remains bounded by upstream's 20-entry LRU, rather than
    // rejecting every fresh mapping after the first eight observations.
    for port in 4001..4101 {
        for suffix in [format!("tcp/{port}"), format!("udp/{port}/quic-v1")] {
            let addr = format!("/ip4/8.8.8.8/{suffix}").parse().unwrap();
            assert!(behaviour.accept_candidate(&addr), "fresh mapping {addr}");
            behaviour.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
                libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &addr },
            ));
        }
    }
    assert!(behaviour.accept_candidate(&"/ip4/9.9.9.9/tcp/4001".parse().unwrap()));
    assert!(!behaviour.accept_candidate(&"/ip4/192.168.1.2/tcp/4001".parse().unwrap()));
}

#[test]
fn relayed_upgrade_policy_enforces_attempt_budget_and_cooldown() {
    let peer = PeerId::random();
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 2);
    behaviour.allow_peer(peer);

    assert!(behaviour.allow_relayed_upgrade(peer, true));
    assert!(!behaviour.allow_relayed_upgrade(peer, true));

    let last = behaviour
        .last_attempt_by_peer
        .get_mut(&peer)
        .expect("last attempt recorded");
    *last = last
        .checked_sub(Duration::from_secs(61))
        .expect("instant subtracts");
    assert!(behaviour.allow_relayed_upgrade(peer, true));

    let last = behaviour
        .last_attempt_by_peer
        .get_mut(&peer)
        .expect("last attempt recorded");
    *last = last
        .checked_sub(Duration::from_secs(61))
        .expect("instant subtracts");
    assert!(!behaviour.allow_relayed_upgrade(peer, true));
}

#[test]
fn relayed_upgrade_policy_rejects_unmarked_peers() {
    let peer = PeerId::random();
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 2);

    assert!(!behaviour.allow_relayed_upgrade(peer, true));
    behaviour.allow_peer(peer);
    assert!(behaviour.allow_relayed_upgrade(peer, true));
}

#[test]
fn dcutr_allowlist_is_bounded() {
    let mut behaviour = DcutrBehaviour::new(PeerId::random(), 60, 2);
    for _ in 0..MAX_ALLOWED_DCUTR_PEERS + 5 {
        behaviour.allow_peer(PeerId::random());
    }

    assert_eq!(behaviour.allowed_peers.len(), MAX_ALLOWED_DCUTR_PEERS);
}
