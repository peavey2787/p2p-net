use std::fs;

use libp2p::{Multiaddr, PeerId};
use p2p_net::connectivity::connection_strategy::build_peer_book_connection_plan;
use p2p_net::{
    DcutrPolicy, DhtProviderState, PeerBook, PeerSource, PublicBootstrapConfig, PublicFallbackMode,
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
fn live_two_node_probe_uses_production_auto_discovery_without_manual_dial() {
    let source =
        fs::read_to_string("examples/live_two_node_probe.rs").expect("read live two-node probe");
    assert!(source.contains("Duration::from_secs(60)"));
    assert!(source.contains("LIVE_TWO_NODE_RESULT=auto_connected"));
    assert!(source.contains("peer_is_application_connected"));
    assert!(source.contains("P2P_LIVE_PROBE_DISABLE_LAN"));
    assert!(source.contains("relay_client_reservations"));
    assert!(source.contains("dcutr_successes"));
    assert!(
        !source.contains("connect_peer("),
        "live two-node validation must not manually inject either peer address",
    );
    assert!(
        !source.contains("discovery.namespace.tags ="),
        "live two-node validation must use the production default discovery namespace",
    );
}

#[test]
fn cross_machine_live_probe_uses_true_default_network_and_namespace() {
    let source = fs::read_to_string("examples/live_single_node_probe.rs")
        .expect("read cross-machine live node probe");
    assert!(source.contains("Duration::from_secs(60)"));
    assert!(source.contains("LIVE_SINGLE_NODE_RESULT=connected"));
    assert!(source.contains("NodeConfig::default()"));
    assert!(
        !source.contains("P2P_LIVE_PROBE_NONCE"),
        "cross-machine validation must not require a shared custom discovery nonce",
    );
    assert!(
        !source.contains("discovery.namespace.tags ="),
        "cross-machine validation must use the production default discovery namespace",
    );
    assert!(
        !source.contains("connect_peer("),
        "cross-machine validation must not manually inject peer addresses",
    );
}

#[test]
fn production_core_contains_lan_and_signed_dht_address_recovery() {
    let lan = fs::read_to_string("crates/connectivity/lan.rs").expect("read LAN discovery");
    for required in [
        "LanDiscoverySocket",
        "reply_requested",
        "ANDROID_EMULATOR_HOST_V4",
        "10, 0, 2, 2",
        "respond(",
        "Emulator probes are request-only",
        "Noise + Identify",
    ] {
        assert!(
            lan.contains(required),
            "missing LAN discovery contract: {required}"
        );
    }

    let records = fs::read_to_string("crates/connectivity/dht/address_records.rs")
        .expect("read signed DHT address records");
    for required in [
        "PeerRecord::new",
        "SignedEnvelope::from_protobuf_encoding",
        "identity mismatch",
        "relay_dial_addr_for_peer",
        "put_record",
        "get_record",
    ] {
        assert!(
            records.contains(required),
            "missing signed DHT address recovery contract: {required}"
        );
    }
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
    assert!(!dht.should_auto_connect_provider_result(&provider));
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
