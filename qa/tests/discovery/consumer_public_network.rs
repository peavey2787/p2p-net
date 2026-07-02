use libp2p::{Multiaddr, PeerId};
use p2p_net::connectivity::connection_strategy::build_peer_book_connection_plan;
use p2p_net::{
    DcutrPolicy, DhtProviderState, PeerBook, PeerSource, PublicBootstrapConfig,
    PublicFallbackMode,
};

#[test]
fn public_fallback_decisions_cover_bootstrap_rendezvous_and_relay() {
    let public_bootstrap = p2p_addr(PeerId::random(), 5101).to_string();
    let public_rendezvous = p2p_addr(PeerId::random(), 5102).to_string();
    let public_relay = p2p_addr(PeerId::random(), 5103).to_string();

    let fallback_only = PublicBootstrapConfig {
        mode: PublicFallbackMode::FallbackOnly,
        bootstrap_seed_peers: vec![public_bootstrap.clone()],
        rendezvous_peers: vec![public_rendezvous.clone()],
        relay_peers: vec![public_relay.clone()],
        auto_connect_discovered_peers: true,
    };

    assert!(fallback_only.bootstrap_decision(0).used);
    assert!(!fallback_only.bootstrap_decision(1).used);
    assert!(fallback_only.rendezvous_decision(0).used);
    assert!(!fallback_only.rendezvous_decision(1).used);
    assert!(fallback_only.relay_decision(0).used);
    assert!(!fallback_only.relay_decision(1).used);

    let always = PublicBootstrapConfig {
        mode: PublicFallbackMode::Always,
        bootstrap_seed_peers: vec![public_bootstrap],
        rendezvous_peers: vec![public_rendezvous],
        relay_peers: vec![public_relay],
        auto_connect_discovered_peers: true,
    };

    assert!(always.bootstrap_decision(1).used);
    assert!(always.rendezvous_decision(1).used);
    assert!(always.relay_decision(1).used);
}

#[test]
fn private_infrastructure_only_disables_public_fallback_and_auto_connect() {
    let private = PublicBootstrapConfig::private_infrastructure_only();

    assert_eq!(private.mode, PublicFallbackMode::Disabled);
    assert!(!private.auto_connect_discovered_peers);
    assert!(!private.bootstrap_decision(0).used);
    assert!(!private.rendezvous_decision(0).used);
    assert!(!private.relay_decision(0).used);
    assert!(private.bootstrap_seed_peers.is_empty());
    assert!(private.rendezvous_peers.is_empty());
    assert!(private.relay_peers.is_empty());
}

#[test]
fn dht_provider_discovery_plans_auto_connect_from_fake_addresses() {
    let provider = PeerId::random();
    let relay = PeerId::random();
    let namespace = "p2p-net/1/hydra-msg/dht-provider-auto-connect";
    let tcp = p2p_addr(provider, 5201);
    let quic = quic_addr(provider, 5202);
    let relayed = relay_addr(relay, provider, 5203);

    let mut dht = DhtProviderState::default();
    assert!(dht.should_auto_connect_provider_result(&provider));
    assert!(dht.mark_auto_connect_waiting_for_addrs(provider));
    assert!(!dht.should_auto_connect_provider_result(&provider));
    assert!(dht.should_auto_connect_after_addr_update(&provider));

    let mut book = PeerBook::default();
    book.record_namespace(provider, namespace, PeerSource::DhtProvider);
    book.record_addr(provider, tcp.clone(), PeerSource::DhtProvider);
    book.record_addr(provider, quic.clone(), PeerSource::DhtProvider);
    book.record_addr(provider, relayed.clone(), PeerSource::PublicRelayDiscovery);

    let plan = build_peer_book_connection_plan(provider, &book, &DcutrPolicy::default());
    let ordered = plan
        .attempts
        .iter()
        .map(|attempt| attempt.addr.to_string())
        .collect::<Vec<_>>();
    let quic = quic.to_string();
    let tcp = tcp.to_string();
    let relayed = relayed.to_string();

    assert_eq!(plan.target_peer, Some(provider));
    assert_eq!(ordered.first().map(String::as_str), Some(quic.as_str()));
    assert!(ordered.contains(&tcp));
    assert_eq!(ordered.last().map(String::as_str), Some(relayed.as_str()));
    assert!(plan.attempt_dcutr_after_relay);

    assert!(dht.mark_auto_connect_attempted(provider));
    assert!(!dht.should_auto_connect_provider_result(&provider));
    assert!(!dht.should_auto_connect_after_addr_update(&provider));
    assert!(dht.mark_auto_connect_failed(&provider));
    assert!(dht.should_auto_connect_provider_result(&provider));
}

#[test]
fn rendezvous_discovery_plans_auto_connect_from_fake_addresses() {
    let peer = PeerId::random();
    let relay = PeerId::random();
    let namespace = "p2p-net/1/hydra-msg/rendezvous-auto-connect";
    let tcp = p2p_addr(peer, 5301);
    let quic = quic_addr(peer, 5302);
    let relayed = relay_addr(relay, peer, 5303);

    let mut book = PeerBook::default();
    book.record_namespace(peer, namespace, PeerSource::PublicRendezvous);
    book.record_addr(peer, tcp.clone(), PeerSource::PublicRendezvous);
    book.record_addr(peer, quic.clone(), PeerSource::PublicRendezvous);
    book.record_addr(peer, relayed.clone(), PeerSource::PublicRelayDiscovery);

    let plan = build_peer_book_connection_plan(peer, &book, &DcutrPolicy::default());
    let ordered = plan
        .attempts
        .iter()
        .map(|attempt| attempt.addr.to_string())
        .collect::<Vec<_>>();
    let quic = quic.to_string();
    let tcp = tcp.to_string();
    let relayed = relayed.to_string();

    assert_eq!(ordered.first().map(String::as_str), Some(quic.as_str()));
    assert!(ordered.contains(&tcp));
    assert_eq!(ordered.last().map(String::as_str), Some(relayed.as_str()));

    let info = book
        .peers()
        .into_iter()
        .find(|candidate| candidate.peer_id == peer.to_string())
        .expect("rendezvous peer in peer book");
    assert!(!info.connected);
    assert!(info.has_source(PeerSource::PublicRendezvous));
    assert!(info.has_source(PeerSource::PublicRelayDiscovery));
    assert!(!info.has_source(PeerSource::Connected));
    assert!(!info.has_source(PeerSource::Manual));
    assert!(info
        .sources
        .iter()
        .map(|source| source.as_str())
        .all(|source| !source.contains("trust") && !source.contains("contact")));
}

fn p2p_addr(peer: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer}")
        .parse()
        .expect("valid TCP peer address")
}

fn quic_addr(peer: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/udp/{port}/quic-v1/p2p/{peer}")
        .parse()
        .expect("valid QUIC peer address")
}

fn relay_addr(relay: PeerId, target: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{relay}/p2p-circuit/p2p/{target}")
        .parse()
        .expect("valid relayed peer address")
}
