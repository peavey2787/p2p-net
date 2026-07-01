#[test]
fn default_dependency_graph_uses_local_no_hickory_dns_patch() {
    let manifest = include_str!("../../Cargo.toml");
    assert!(
        manifest.contains("dns = [\"libp2p/dns\"]"),
        "WebSocket support requires libp2p/dns to activate the local patched libp2p-dns adapter"
    );
    assert!(
        manifest.contains("libp2p-dns = { path = \"external/vendor/libp2p-dns\" }"),
        "libp2p-dns must be patched to the local no-Hickory adapter"
    );
}

#[test]
fn upstream_mdns_is_policy_patched_to_noop_placeholder() {
    let manifest = include_str!("../../Cargo.toml");
    assert!(
        manifest.contains("libp2p-mdns = { path = \"external/vendor/libp2p-mdns-placeholder\" }"),
        "disallowed upstream libp2p-mdns must be patched away from crates.io"
    );
}

#[test]
fn local_dns_patch_does_not_hide_doh_or_hickory() {
    let dns_manifest = include_str!("../../external/vendor/libp2p-dns/Cargo.toml");
    let dns_source = include_str!("../../external/vendor/libp2p-dns/src/lib.rs");

    assert!(
        !dns_manifest.contains("hickory") && !dns_source.contains("hickory-"),
        "local libp2p-dns patch must not depend on or reference Hickory packages"
    );
    assert!(
        !dns_manifest.contains("reqwest") && !dns_source.contains("cloudflare-dns.com"),
        "transport-level DNS adapter must not contain a hidden DoH provider"
    );
    assert!(
        dns_source.contains("DnsaddrRequiresConfiguredPreresolver"),
        "/dnsaddr must be handled by p2p-net's configured pre-resolver, not by the transport adapter"
    );
}
