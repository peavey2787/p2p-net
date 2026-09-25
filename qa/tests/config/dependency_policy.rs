#[test]
fn crates_io_manifest_is_publishable_without_manifest_patches() {
    let manifest = include_str!("../../../Cargo.toml");
    let cargo_config = include_str!("../../../.cargo/config.toml");
    let lockfile = include_str!("../../../Cargo.lock");
    let dns_patch = include_str!("../../../external/libp2p-dns/Cargo.toml");
    let mdns_patch = include_str!("../../../external/libp2p-mdns-placeholder/Cargo.toml");
    // The native transport (the only one with a WebSocket listener) is split out per target.
    let transport_source = include_str!("../../../crates/stack/transport/native.rs");

    assert!(manifest.contains("publish = true"));
    assert!(manifest.contains("rust-version = \"1.98\""));
    assert!(manifest.contains("members = [\"apps/android/native\"]"));
    assert!(manifest.contains("exclude = [\"qa/fuzz\", \"external/libp2p-webrtc\"]"));
    assert!(manifest.contains("\"external/libp2p-webrtc/**\""));
    assert!(manifest.contains("dns = []"));
    assert!(manifest.contains("libp2p-websocket = \"0.46.0\""));
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

    // libp2p-swarm 0.48.0 requires wasm-bindgen-futures =0.4.58. Keep the
    // complete direct browser ABI family pinned to the matching wasm-bindgen
    // generation. futures-timer 3.0.3 is intentional: 3.0.4 pulls gloo-timers
    // 0.4.0, which requires js-sys >=0.3.91 and conflicts with this ABI family.
    for browser_pin in [
        "wasm-bindgen = \"=0.2.108\"",
        "wasm-bindgen-futures = \"=0.4.58\"",
        "serde-wasm-bindgen = \"=0.6.5\"",
        "js-sys = \"=0.3.85\"",
        "futures-timer = { version = \"=3.0.3\", features = [\"wasm-bindgen\"] }",
        "getrandom = { version = \"=0.2.17\", features = [\"js\"] }",
        "web-sys = { version = \"=0.3.85\", features = [",
    ] {
        assert!(
            manifest.contains(browser_pin),
            "browser/WASM dependency family must stay aligned with libp2p-swarm 0.48.0: {browser_pin}"
        );
    }

    // The root now owns native WebSocket transport directly. A stale lockfile
    // that still lists libp2p-websocket under the libp2p package makes Cargo 1.98
    // require a lockfile rewrite and breaks every strict --locked path.
    let libp2p_block = lockfile
        .split("[[package]]")
        .find(|block| block.contains("name = \"libp2p\"") && block.contains("version = \"0.57.0\""))
        .expect("Cargo.lock missing libp2p 0.57.0");
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
    for browser_dependency in [
        "\"futures-timer\"",
        "\"getrandom 0.2.17\"",
        "\"js-sys\"",
        "\"serde-wasm-bindgen\"",
        "\"wasm-bindgen\"",
        "\"wasm-bindgen-futures\"",
        "\"web-sys\"",
    ] {
        assert!(
            root_block.contains(browser_dependency),
            "p2p-net browser dependency missing from Cargo.lock root package: {browser_dependency}"
        );
    }
    for browser_package in [
        "name = \"serde-wasm-bindgen\"\nversion = \"0.6.5\"",
        "name = \"wasm-bindgen-futures\"\nversion = \"0.4.58\"",
        "name = \"wasm-bindgen\"\nversion = \"0.2.108\"",
    ] {
        assert!(
            lockfile.contains(browser_package),
            "required pinned browser package missing from Cargo.lock: {browser_package}"
        );
    }
    let futures_timer_block = lockfile
        .split("[[package]]")
        .find(|block| {
            block.contains("name = \"futures-timer\"") && block.contains("version = \"3.0.3\"")
        })
        .expect("futures-timer 3.0.3 package missing from Cargo.lock");
    assert!(
        futures_timer_block.contains("\"gloo-timers\"")
            || futures_timer_block.contains("\"gloo-timers 0.2.6\""),
        "futures-timer 3.0.3 WASM lock edge to gloo-timers is missing"
    );
    assert!(
        futures_timer_block.contains("\"send_wrapper 0.4.0\"")
            || (futures_timer_block.contains("\"send_wrapper\"")
                && !lockfile.contains("name = \"send_wrapper\"\nversion = \"0.6.0\"")),
        "futures-timer 3.0.3 WASM lock edge to send_wrapper 0.4.0 is missing"
    );
    for wasm_timer_package in [
        "name = \"gloo-timers\"\nversion = \"0.2.6\"",
        "name = \"send_wrapper\"\nversion = \"0.4.0\"",
    ] {
        assert!(
            lockfile.contains(wasm_timer_package),
            "required futures-timer WASM package missing from Cargo.lock: {wasm_timer_package}"
        );
    }
    assert!(
        !lockfile.contains("name = \"futures-timer\"\nversion = \"3.0.4\""),
        "futures-timer 3.0.4 is incompatible with libp2p 0.57's pinned wasm-bindgen ABI generation"
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
        "libp2p-webrtc = { package = \"p2p-net-webrtc\", version = \"0.1.0\", path = \"external/libp2p-webrtc\", features = [\"tokio\", \"pem\"] }"
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
    let transports = [
        include_str!("../../../crates/stack/transport.rs"),
        include_str!("../../../crates/stack/transport/native.rs"),
        include_str!("../../../crates/stack/transport/wasm.rs"),
    ];
    let handle = include_str!("../../../crates/node/handle.rs");

    for transport in transports {
        assert!(!transport.contains(".with_dns()"));
    }
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
        manifest.contains("webrtc = { version = \"0.17.0\", optional = true }"),
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

#[test]
fn playwright_browser_target_isolation_is_explicit() {
    let manifest = include_str!("../../../Cargo.toml");
    let windows_runner = include_str!("../../../run-full-validation.cmd");
    let linux_runner = include_str!("../../../run-full-validation.sh");
    let playwright_runner = include_str!("../../browser/run-playwright.cjs");
    let browser_storage = include_str!("../../../crates/platform/wasm.rs");
    let windows_evidence = include_str!("../../evidence/run-validation-with-evidence.ps1");
    let linux_evidence = include_str!("../../evidence/run-validation-with-evidence.sh");
    let browser_compile_gate = include_str!("../wasm/browser_wasm.rs");
    let browser_qa_exports = include_str!("../../../crates/wasm/qa.rs");
    let browser_node = include_str!("../../../crates/wasm.rs");

    assert!(manifest.contains("default = [\"dns\", \"native-tests\"]"));
    assert!(manifest.contains("native-tests = []"));
    assert!(manifest.contains("browser-tests = []"));

    let test_blocks: Vec<_> = manifest.split("[[test]]").skip(1).collect();
    assert!(
        !test_blocks.is_empty(),
        "integration tests must remain registered"
    );
    for block in test_blocks {
        let is_browser = block.contains("name = \"browser_wasm\"");
        if is_browser {
            assert!(
                block.contains("required-features = [\"browser-tests\"]"),
                "browser_wasm must require only the browser test selector"
            );
        } else {
            assert!(
                block.contains("required-features = [\"native-tests\"]"),
                "every non-browser integration test must require the native test selector: {block}"
            );
        }
    }

    assert!(
        windows_runner.contains("call npm install --prefix"),
        "Windows batch validation must CALL npm.cmd so control returns after Playwright installation"
    );

    for runner in [windows_runner, linux_runner] {
        assert!(
            runner.contains("PLAYWRIGHT_VERSION=1.63.0")
                || runner.contains("PLAYWRIGHT_VERSION=\"1.63.0\""),
            "browser QA must pin the Playwright tool version"
        );
        assert!(
            runner.contains("playwright@") && runner.contains("install chromium firefox"),
            "browser QA must use Playwright-managed Chromium and Firefox"
        );
        assert!(
            runner.contains("--test browser_wasm --locked")
                && runner.contains("wasm-pack build . --dev --target web")
                && runner.contains("--no-default-features --features dns,browser-tests"),
            "Playwright browser QA must type-check the browser tests and build only the browser-selected WASM facade"
        );
        assert!(
            !runner.contains("chromedriver")
                && !runner.contains("--chrome")
                && !runner.contains("wasm-pack test"),
            "browser QA must not depend on ChromeDriver or wasm-bindgen-test WebDriver execution"
        );
    }

    for evidence_wrapper in [windows_evidence, linux_evidence] {
        assert!(
            evidence_wrapper.contains("P2P_VALIDATION_COMPLETION_SENTINEL")
                && evidence_wrapper.contains("launcher-complete.txt"),
            "evidence wrappers must reject premature zero-exit runs that never reach launcher completion"
        );
    }
    for runner in [windows_runner, linux_runner] {
        assert!(
            runner.contains("P2P_VALIDATION_COMPLETION_SENTINEL"),
            "validation launchers must emit the evidence completion sentinel only on their final success path"
        );
    }

    for required in [
        "const { chromium, firefox } = require('playwright');",
        "STEP_TIMEOUT_MS",
        "withTimeout",
        "RUN: Playwright",
        "qaStorageWrite",
        "page.reload",
        "qaStorageRead",
        "qaProfileLifecycle",
    ] {
        assert!(
            playwright_runner.contains(required),
            "Playwright browser runner missing `{required}`"
        );
    }
    assert!(
        browser_storage.matches("futures::try_join!").count() >= 2
            && browser_storage.contains("INDEXED_DB_OPERATION_TIMEOUT")
            && browser_storage.contains("crate::runtime::timeout"),
        "IndexedDB request and transaction completion handlers must be armed together and time-bounded"
    );
    assert!(
        browser_compile_gate.contains("qa_storage_write")
            && browser_compile_gate.contains("qa_storage_read")
            && browser_compile_gate.contains("qa_profile_lifecycle"),
        "wasm32 compile gate must keep the Playwright-only QA exports type-checked"
    );
    assert!(
        browser_qa_exports.contains("QA_PROFILE_STAGE_TIMEOUT")
            && browser_qa_exports.contains("private_infrastructure_only")
            && browser_qa_exports.contains("config.discovery.dht.enabled = false")
            && browser_qa_exports.contains("config.dnsaddr.enabled = false"),
        "browser profile lifecycle QA must be offline-deterministic and stage-time-bounded"
    );
    assert!(
        browser_node.contains("profile_lock_release")
            && browser_node.contains(".release()")
            && browser_node.contains("release(); return request;")
            && playwright_runner.contains("error && error.kind"),
        "browser profile shutdown must await lock release and Playwright must preserve structured QA failures"
    );
}
