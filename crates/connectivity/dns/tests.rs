use super::*;

#[test]
fn detects_dnsaddr_as_resolvable_dns_kind() {
    let addr: Multiaddr = concat!(
        "/dnsaddr/example.com/p2p/",
        "12D3KooWQnQJ6pVwbQzqJ2m6SfqxJrQx7dUMJzE3p9Fv4v2o2hQZ"
    )
    .parse()
    .unwrap();
    assert!(has_dnsaddr(&addr));
    assert!(has_any_dns(&addr));
}

#[test]
fn detects_plain_dns_as_resolvable() {
    let addr: Multiaddr = "/dns4/example.com/tcp/4001".parse().unwrap();
    assert!(has_resolvable_dns(&addr));
    assert!(!has_dnsaddr(&addr));
}

#[test]
fn dnsaddr_query_name_adds_prefix() {
    assert_eq!(
        dnsaddr_query_name("bootstrap.libp2p.io"),
        "_dnsaddr.bootstrap.libp2p.io."
    );
    assert_eq!(
        dnsaddr_query_name("bootstrap.libp2p.io."),
        "_dnsaddr.bootstrap.libp2p.io."
    );
}

#[test]
fn dnsaddr_peer_suffix_filters_instead_of_appending_duplicate_p2p() {
    let requested: Multiaddr = concat!(
        "/dnsaddr/bootstrap.libp2p.io/p2p/",
        "QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN"
    )
    .parse()
    .unwrap();
    let (_, suffix) = split_first_dnsaddr(&requested).unwrap();
    let matching: Multiaddr = concat!(
        "/ip4/147.75.83.83/tcp/4001/p2p/",
        "QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN"
    )
    .parse()
    .unwrap();
    let different: Multiaddr = concat!(
        "/ip4/147.75.83.83/tcp/4001/p2p/",
        "QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa"
    )
    .parse()
    .unwrap();

    assert!(multiaddr_ends_with(&matching, &suffix));
    assert!(!multiaddr_ends_with(&different, &suffix));
    assert_eq!(
        matching
            .iter()
            .filter(|protocol| matches!(protocol, Protocol::P2p(_)))
            .count(),
        1
    );
}
