#[test]
fn crates_io_manifest_is_publishable_without_manifest_patches() {
    let manifest = include_str!("../../../Cargo.toml");
    let cargo_config = include_str!("../../../.cargo/config.toml");
    let lockfile = include_str!("../../../Cargo.lock");
    let dns_patch = include_str!("../../../external/libp2p-dns/Cargo.toml");
    let mdns_patch = include_str!("../../../external/libp2p-mdns-placeholder/Cargo.toml");
    let transport_source = include_str!("../../../crates/stack/transport.rs");

    assert!(manifest.contains("publish = true"));
    assert!(manifest.contains("rust-version = \"1.98\""));
    assert!(manifest.contains("members = [\"apps/android/native\"]"));
    assert!(manifest.contains("exclude = [\"qa/fuzz\", \"external/libp2p-webrtc\"]"));
    assert!(manifest.contains("\"external/libp2p-webrtc/**\""));
    assert!(manifest.contains("dns = []"));
    assert!(manifest.contains("libp2p-websocket = \"0.45.1\""));
    assert!(
        !manifest.lines().any(|line| line.trim() == "\"websocket\","),
        "top-level libp2p websocket feature must stay disabled; p2p-net uses libp2p-websocket directly so libp2p does not compile its DNS-coupled websocket builder"
    );
    assert!(
        transport_source.contains("use libp2p_websocket as websocket;"),
        "WebSocket transport must use the direct libp2p-websocket crate"
    );
    assert!(
        !manifest.contains("[patch.crates-io]") && !manifest.contains("libp2p/dns"),
        "the published p2p-net dependency graph must not require repository-local DNS/mDNS patches"
    );

    // Cargo resolves weak optional dependency feature references while creating
    // a workspace lockfile. Keep those resolution-only libp2p packages local so
    // the committed lock remains stable without reintroducing Hickory.
    for required in [
        "[patch.crates-io]",
        "libp2p-dns = { path = \"external/libp2p-dns\" }",
        "libp2p-mdns = { path = \"external/libp2p-mdns-placeholder\" }",
    ] {
        assert!(
            cargo_config.contains(required),
            "Cargo config missing `{required}`"
        );
    }
    assert!(
        !cargo_config.contains("path = \"../external/"),
        "Cargo config paths are relative to the repository directory above .cargo; ../external escapes the repo"
    );
    assert!(dns_patch.contains("name = \"libp2p-dns\""));
    assert!(dns_patch.contains("publish = false"));
    assert!(!dns_patch.contains("hickory"));
    assert!(mdns_patch.contains("name = \"libp2p-mdns\""));
    assert!(mdns_patch.contains("publish = false"));
    assert!(lockfile.contains("name = \"libp2p-dns\""));
    assert!(lockfile.contains("name = \"libp2p-mdns\""));

    // The root now owns WebSocket transport directly. A stale lockfile that
    // still lists libp2p-websocket under the libp2p package makes Cargo 1.98
    // require a lockfile rewrite and breaks every strict --locked path.
    let libp2p_block = lockfile
        .split("[[package]]")
        .find(|block| block.contains("name = \"libp2p\"") && block.contains("version = \"0.56.0\""))
        .expect("Cargo.lock missing libp2p 0.56.0");
    assert!(
        !libp2p_block.contains("\"libp2p-websocket\""),
        "libp2p-websocket must not remain feature-resolved under libp2p after disabling libp2p's websocket feature"
    );
    let root_block = lockfile
        .split("[[package]]")
        .find(|block| block.contains("name = \"p2p-net\"") && block.contains("version = \"0.1.0\""))
        .expect("Cargo.lock missing p2p-net 0.1.0");
    assert!(
        root_block.contains("\"libp2p-websocket\""),
        "p2p-net must retain its direct libp2p-websocket lock dependency"
    );
    assert!(
        !lockfile.contains("name = \"hickory-"),
        "resolution-only DNS/mDNS lock entries must remain Hickory-free"
    );
}

#[test]
fn hardened_webrtc_transport_is_a_publishable_companion_crate() {
    let manifest = include_str!("../../../Cargo.toml");
    let webrtc_manifest = include_str!("../../../external/libp2p-webrtc/Cargo.toml");
    let upgrade_source = include_str!("../../../external/libp2p-webrtc/src/tokio/upgrade.rs");
    let mux_source = include_str!("../../../external/libp2p-webrtc/src/tokio/udp_mux.rs");
    let connection_source = include_str!("../../../external/libp2p-webrtc/src/tokio/connection.rs");
    let smoke_source = include_str!("../../../external/libp2p-webrtc/tests/smoke.rs");

    assert!(manifest.contains(
        "libp2p-webrtc = { package = \"p2p-net-webrtc\", version = \"0.1.0\", path = \"external/libp2p-webrtc\", features = [\"tokio\"] }"
    ));
    for required in [
        "name = \"p2p-net-webrtc\"",
        "version = \"0.1.0\"",
        "publish = true",
        "name = \"libp2p_webrtc\"",
    ] {
        assert!(
            webrtc_manifest.contains(required),
            "publishable WebRTC companion missing `{required}`"
        );
    }
    assert!(
        !webrtc_manifest.contains("quickcheck")
            && !webrtc_manifest.contains("tracing-subscriber")
            && !webrtc_manifest.contains("features = [\"full\"]"),
        "publishable companion tests must not add package-only/randomized dependency resolution"
    );
    assert!(smoke_source.contains("[(1, 1), (2, 3), (4, 2)]"));
    assert!(smoke_source.contains("exercise_concurrent_connections_and_streams"));
    assert!(upgrade_source.contains("set_srtp_protection_profiles"));
    assert!(!upgrade_source.contains("Srtp_Aead_Aes_256_Gcm"));
    for required in [
        "MAX_PENDING_NEW_ADDRS",
        "PENDING_NEW_ADDR_TTL",
        "PendingNewAddrs",
        "removed_conn.close()",
    ] {
        assert!(
            mux_source.contains(required),
            "missing WebRTC guard `{required}`"
        );
    }
    for required in [
        "CONNECTION_SETUP_TIMEOUT",
        "MuxConnCleanup",
        "PendingPeerConnection",
        "remove_conn_by_ufrag",
    ] {
        assert!(
            upgrade_source.contains(required),
            "missing WebRTC cleanup guard `{required}`"
        );
    }
    assert!(
        connection_source.contains("impl Drop for Connection")
            && connection_source.contains("peer_conn.close().await")
            && connection_source
                .matches("Arc::downgrade(&data_channel)")
                .count()
                >= 2
    );
}

#[test]
fn dns_resolution_is_owned_by_p2p_net_including_manual_dials() {
    let dns = include_str!("../../../crates/connectivity/dns.rs");
    let transport = include_str!("../../../crates/stack/transport.rs");
    let handle = include_str!("../../../crates/node/handle.rs");

    assert!(!transport.contains(".with_dns()"));
    assert!(dns.contains("pub(crate) async fn resolve_dial_multiaddrs"));
    assert!(dns.contains("tokio::net::lookup_host"));
    assert!(dns.contains("DEFAULT_DNSADDR_DOH_ENDPOINT"));
    assert!(handle.contains("resolve_dial_multiaddrs(&addr, &self.dnsaddr).await?"));
}

#[test]
fn direct_webrtc_probe_stays_on_the_audited_dependency_generation() {
    let manifest = include_str!("../../../Cargo.toml");
    let lockfile = include_str!("../../../Cargo.lock");
    let probe = include_str!("../../../examples/live_webrtc_oob_probe.rs");

    assert!(
        manifest.contains("webrtc = { version = \"0.12.0\", optional = true }"),
        "the direct WebRTC probe must share the audited webrtc-rs generation used by p2p-net-webrtc"
    );
    for vulnerable_package in [
        "name = \"webrtc\"\nversion = \"0.8.0\"",
        "name = \"curve25519-dalek\"\nversion = \"3.2.0\"",
        "name = \"ring\"\nversion = \"0.16.20\"",
        "name = \"rustls\"\nversion = \"0.19.1\"",
        "name = \"webpki\"\nversion = \"0.21.4\"",
    ] {
        assert!(
            !lockfile.contains(vulnerable_package),
            "the obsolete WebRTC dependency chain must not return to Cargo.lock: {vulnerable_package}"
        );
    }
    assert!(probe.contains("set_srtp_protection_profiles"));
    assert!(!probe.contains("Srtp_Aead_Aes_256_Gcm"));
}

#[test]
fn local_webrtc_transport_declares_runtime_cleanup_support() {
    let webrtc_manifest = include_str!("../../../external/libp2p-webrtc/Cargo.toml");
    assert!(
        webrtc_manifest.contains("features = [\"net\", \"rt\", \"time\"]"),
        "the hardened WebRTC transport uses Tokio runtime cleanup and must declare the rt feature"
    );
}
