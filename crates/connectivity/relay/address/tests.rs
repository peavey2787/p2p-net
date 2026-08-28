use super::*;

#[test]
fn builds_relay_reservation_address() {
    let relay_peer = PeerId::random();
    let relay_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay_peer}")
        .parse()
        .unwrap();
    let reservation = relay_reservation_addr(&relay_addr).unwrap();
    assert_eq!(
        reservation.to_string(),
        format!("/ip4/127.0.0.1/tcp/4001/p2p/{relay_peer}/p2p-circuit")
    );
    assert_eq!(relay_peer_id(&reservation), Some(relay_peer));
    assert!(is_p2p_circuit_addr(&reservation));
}

#[test]
fn relay_reservation_becomes_target_bound_dial_route() {
    let relay_peer = PeerId::random();
    let target = PeerId::random();
    let reservation: Multiaddr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay_peer}/p2p-circuit")
        .parse()
        .unwrap();
    let route = relay_dial_addr_for_peer(&reservation, target).expect("route");
    assert_eq!(
        route.to_string(),
        format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay_peer}/p2p-circuit/p2p/{target}")
    );
}

#[test]
fn reservation_address_rejects_non_relay_addr() {
    let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
    assert!(relay_reservation_addr(&addr).is_none());
}
