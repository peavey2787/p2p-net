use libp2p::{Multiaddr, PeerId};
use p2p_net::{
    build_connection_plan, DcutrPolicy, DiscoveryConfig, DiscoveryNamespaceConfig, PeerBook,
    PeerSource, PublicBootstrapConfig, PublicFallbackMode,
};

#[test]
fn joe_and_alice_recover_contact_visibility_from_shared_hashed_namespace() {
    let raw_contact_tag = "IAmJoeTheRealJoeYouWant213423432";
    let joe = PeerId::random();
    let joe_direct = direct_addr(joe, 4101);

    let joe_cfg = hydra_discovery(raw_contact_tag);
    let alice_cfg = hydra_discovery(raw_contact_tag);
    let joe_namespaces = joe_cfg.rendezvous_namespaces(42).expect("joe namespace");
    let alice_namespaces = alice_cfg
        .rendezvous_namespaces(42)
        .expect("alice namespace");

    assert_eq!(joe_namespaces, alice_namespaces);
    assert_eq!(joe_namespaces.len(), 1);
    assert!(joe_namespaces[0].starts_with("p2p-net/42/hydra-msg/"));
    assert!(!joe_namespaces[0].contains(raw_contact_tag));
    assert!(!joe_namespaces[0].to_ascii_lowercase().contains("joe"));

    let mut alice_peer_book = PeerBook::default();
    alice_peer_book.record_namespace(joe, joe_namespaces[0].clone(), PeerSource::DhtProvider);
    alice_peer_book.record_addr(joe, joe_direct.clone(), PeerSource::Rendezvous);

    let peers = alice_peer_book.peers();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].peer_id, joe.to_string());
    assert!(!peers[0].connected);
    assert_eq!(peers[0].addresses, vec![joe_direct.to_string()]);
    assert_eq!(
        peers[0].namespace.as_deref(),
        Some(joe_namespaces[0].as_str())
    );
    assert!(peers[0].has_source(PeerSource::DhtProvider));
    assert!(peers[0].has_source(PeerSource::Rendezvous));
}

#[test]
fn public_fallback_only_participates_when_policy_allows_it() {
    let public_seed = direct_addr(PeerId::random(), 4201).to_string();
    let public_relay = relay_addr(PeerId::random(), PeerId::random(), 4202).to_string();

    let disabled = PublicBootstrapConfig {
        mode: PublicFallbackMode::Disabled,
        bootstrap_seed_peers: vec![public_seed.clone()],
        rendezvous_peers: Vec::new(),
        relay_peers: vec![public_relay.clone()],
        auto_connect_discovered_peers: false,
    };
    assert!(!disabled.bootstrap_decision(0).used);
    assert!(!disabled.relay_decision(0).used);

    let fallback_only = PublicBootstrapConfig {
        mode: PublicFallbackMode::FallbackOnly,
        bootstrap_seed_peers: vec![public_seed.clone()],
        rendezvous_peers: Vec::new(),
        relay_peers: vec![public_relay.clone()],
        auto_connect_discovered_peers: true,
    };
    assert!(!fallback_only.bootstrap_decision(1).used);
    assert!(fallback_only.bootstrap_decision(0).used);
    assert!(!fallback_only.relay_decision(1).used);
    assert!(fallback_only.relay_decision(0).used);

    let always = PublicBootstrapConfig {
        mode: PublicFallbackMode::Always,
        bootstrap_seed_peers: vec![public_seed],
        rendezvous_peers: Vec::new(),
        relay_peers: vec![public_relay],
        auto_connect_discovered_peers: true,
    };
    assert!(always.bootstrap_decision(1).used);
    assert!(always.relay_decision(1).used);
}

#[test]
fn discovered_but_not_connected_peers_are_returned_for_app_resurrection() {
    let alice = PeerId::random();
    let joe = PeerId::random();
    let alice_namespace = hydra_namespace("alice-shared-contact-card", 7);
    let joe_namespace = hydra_namespace("joe-shared-contact-card", 7);

    let mut book = PeerBook::default();
    book.record_namespace(alice, alice_namespace.clone(), PeerSource::Rendezvous);
    book.record_addr(alice, direct_addr(alice, 4301), PeerSource::Rendezvous);
    book.record_namespace(joe, joe_namespace.clone(), PeerSource::DhtProvider);
    book.record_addr(joe, direct_addr(joe, 4302), PeerSource::DhtProvider);

    let peers = book.peers();
    assert_eq!(book.connected_count(), 0);
    assert_eq!(book.discovered_count(), 2);
    assert_eq!(peers.len(), 2);
    assert!(peers.iter().all(|peer| !peer.connected));
    assert!(peers
        .iter()
        .any(|peer| peer.namespace.as_deref() == Some(alice_namespace.as_str())));
    assert!(peers
        .iter()
        .any(|peer| peer.namespace.as_deref() == Some(joe_namespace.as_str())));
}

#[test]
fn connect_peer_plan_consumes_resurrected_peer_book_addresses() {
    let joe = PeerId::random();
    let requested_stale = direct_addr(joe, 4401);
    let cached_quic = quic_addr(joe, 4402);
    let rendezvous_tcp = direct_addr(joe, 4403);
    let relay = relay_addr(PeerId::random(), joe, 4404);
    let namespace = hydra_namespace("joe-shared-contact-card", 9);

    let mut book = PeerBook::default();
    book.record_namespace(joe, namespace, PeerSource::DhtProvider);
    book.record_addr(joe, rendezvous_tcp.clone(), PeerSource::Rendezvous);
    book.record_addr(joe, cached_quic.clone(), PeerSource::PeerCache);
    book.record_addr(joe, relay.clone(), PeerSource::RelayDiscovery);

    let plan = build_connection_plan(requested_stale.clone(), &book, &DcutrPolicy::default());
    let ordered = plan
        .attempts
        .iter()
        .map(|attempt| attempt.addr.to_string())
        .collect::<Vec<_>>();

    assert_eq!(plan.target_peer, Some(joe));
    assert_eq!(ordered[0], cached_quic.to_string());
    assert!(ordered.contains(&requested_stale.to_string()));
    assert!(ordered.contains(&rendezvous_tcp.to_string()));
    assert_eq!(ordered.last(), Some(&relay.to_string()));
    assert!(plan.attempt_dcutr_after_relay);
    assert!(plan.keep_relay_fallback);
}

fn hydra_namespace(tag: &str, network_id: u32) -> String {
    hydra_discovery(tag)
        .rendezvous_namespaces(network_id)
        .expect("namespace")
        .into_iter()
        .next()
        .expect("namespace present")
}

fn hydra_discovery(tag: &str) -> DiscoveryConfig {
    DiscoveryConfig {
        namespace: DiscoveryNamespaceConfig {
            app_id: "hydra-msg".to_string(),
            tags: vec![tag.to_string()],
            ..DiscoveryNamespaceConfig::default()
        },
        ..DiscoveryConfig::default()
    }
}

fn direct_addr(peer: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer}")
        .parse()
        .expect("valid direct addr")
}

fn quic_addr(peer: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/udp/{port}/quic-v1/p2p/{peer}")
        .parse()
        .expect("valid quic addr")
}

fn relay_addr(relay: PeerId, target: PeerId, port: u16) -> Multiaddr {
    format!("/ip4/127.0.0.1/tcp/{port}/p2p/{relay}/p2p-circuit/p2p/{target}")
        .parse()
        .expect("valid relay addr")
}
