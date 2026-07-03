use libp2p::{Multiaddr, PeerId};
use p2p_net::{PeerBook, PeerSource};

fn peer_addr(peer: PeerId) -> Multiaddr {
    format!("/ip4/127.0.0.1/tcp/4001/p2p/{peer}")
        .parse()
        .expect("valid peer multiaddr")
}

#[test]
fn peer_book_merges_discovery_sources_for_get_peers() {
    let peer = PeerId::random();
    let mut book = PeerBook::default();

    book.record_addr(peer, peer_addr(peer), PeerSource::PeerCache);
    book.record_namespace(peer, "p2p-net/1/hydra/abc123", PeerSource::DhtProvider);
    book.record_connected(peer, None);

    let peers = book.peers();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].peer_id, peer.to_string());
    assert!(peers[0].connected);
    assert!(peers[0].has_source(PeerSource::PeerCache));
    assert!(peers[0].has_source(PeerSource::DhtProvider));
    assert!(peers[0].has_source(PeerSource::Connected));
    assert_eq!(
        peers[0].namespace.as_deref(),
        Some("p2p-net/1/hydra/abc123")
    );
}

#[test]
fn peer_book_reports_discovered_peers_before_connection() {
    let peer = PeerId::random();
    let mut book = PeerBook::default();

    book.record_namespace(peer, "p2p-net/1/hydra/abc123", PeerSource::Rendezvous);

    let peers = book.peers();
    assert_eq!(book.len(), 1);
    assert_eq!(book.connected_count(), 0);
    assert_eq!(book.discovered_count(), 1);
    assert!(!peers[0].connected);
    assert!(peers[0].has_source(PeerSource::Rendezvous));
}

#[test]
fn peer_book_distinguishes_public_fallback_sources() {
    let peer = PeerId::random();
    let mut book = PeerBook::default();

    book.record_addr(peer, peer_addr(peer), PeerSource::PublicRendezvous);
    book.record_peer(peer, PeerSource::PublicBootstrapSeed);
    book.record_peer(peer, PeerSource::PublicRelayDiscovery);

    let peers = book.peers();
    assert_eq!(peers.len(), 1);
    assert!(peers[0].has_source(PeerSource::PublicRendezvous));
    assert!(peers[0].has_source(PeerSource::PublicBootstrapSeed));
    assert!(peers[0].has_source(PeerSource::PublicRelayDiscovery));
    assert!(!peers[0].has_source(PeerSource::Rendezvous));
}
