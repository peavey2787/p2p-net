use std::path::PathBuf;

use libp2p::PeerId;
use p2p_net::connectivity::identity::load_or_create_identity_key_with_storage;
use p2p_net::connectivity::peer_cache::{
    load_entries_with_storage, record_seen_peer_addr_with_storage,
};
#[cfg(unix)]
use p2p_net::DesktopPlatformRuntime;
use p2p_net::{
    DiscoveryConfig, MemoryNodeStorage, MobilePlatformRuntime, NetworkReachability, NodeConfig,
    NodeRole, NodeStorage, PlatformKind, PlatformRuntime,
};

#[test]
fn mobile_runtime_hints_resolve_auto_profile_to_mobile_lite() {
    let cfg = NodeConfig::default();
    let runtime = MobilePlatformRuntime::android(Some(PathBuf::from("/app/private/p2p-net")));

    let report = cfg.environment_report_with_runtime(&runtime);
    assert_eq!(report.platform, PlatformKind::Android);
    assert_eq!(report.reachability, NetworkReachability::PrivateNat);
    assert!(!report.can_listen_tcp);
    assert!(!report.can_listen_quic);
    assert!(!report.can_accept_inbound);
    assert!(report.battery_sensitive);
    assert!(report.background_restricted);

    let resolved = cfg
        .try_resolved_for_environment(&report)
        .expect("mobile runtime should resolve");
    assert_eq!(resolved.role, NodeRole::MobileLite);
    assert!(!resolved.should_listen);
    assert!(resolved.enabled_behaviours.relay_client);
    assert!(resolved.enabled_behaviours.dcutr);
}

#[test]
fn config_environment_overrides_runtime_hints() {
    let mut cfg = NodeConfig::default();
    cfg.environment.platform_hint = Some(PlatformKind::Linux);
    cfg.environment.can_listen_tcp = Some(true);
    cfg.environment.background_restricted = Some(false);

    let runtime = MobilePlatformRuntime::ios(Some(PathBuf::from("/ios/container")));
    let report = cfg.environment_report_with_runtime(&runtime);

    assert_eq!(report.platform, PlatformKind::Linux);
    assert!(report.can_listen_tcp);
    assert!(!report.background_restricted);
}

#[test]
fn identity_and_peer_cache_can_use_abstract_storage() {
    let storage = MemoryNodeStorage::new();
    let first = load_or_create_identity_key_with_storage("identity", &storage)
        .expect("create storage-backed identity");
    let second = load_or_create_identity_key_with_storage("identity", &storage)
        .expect("reload storage-backed identity");
    assert_eq!(PeerId::from(first.public()), PeerId::from(second.public()));

    let peer = PeerId::random();
    let addr = format!("/ip4/203.0.113.10/tcp/4001/p2p/{peer}")
        .parse()
        .expect("test multiaddr");
    let cfg = DiscoveryConfig {
        peer_cache_path: "peer-cache".to_string(),
        ..DiscoveryConfig::default()
    };

    record_seen_peer_addr_with_storage(&cfg, &peer, &addr, &storage);
    let entries = load_entries_with_storage(&cfg, &storage);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].peer_id, peer.to_string());
    assert_eq!(entries[0].addr, addr.to_string());
    assert_eq!(storage.storage_kind(), "memory");
}

#[test]
fn platform_runtime_exposes_storage_and_lifecycle_boundaries() {
    let runtime = MobilePlatformRuntime::ios(Some(PathBuf::from("/app/group/p2p-net")))
        .with_listen_capability(false, true)
        .with_background_restricted(true)
        .with_battery_sensitive(true)
        .with_inbound_capability(Some(false));

    assert_eq!(runtime.runtime_name(), "mobile");
    assert_eq!(runtime.platform_kind(), PlatformKind::Ios);
    assert_eq!(
        runtime.default_data_dir(),
        Some(PathBuf::from("/app/group/p2p-net"))
    );
    assert!(!runtime.can_listen_tcp());
    assert!(runtime.can_listen_quic());
    assert_eq!(runtime.can_accept_inbound(), Some(false));
    assert!(runtime.is_background_restricted());
}

#[cfg(unix)]
#[test]
fn desktop_secret_storage_is_create_only_and_private() {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!("p2p-net-secret-test-{}", PeerId::random()));
    std::fs::create_dir_all(&root).expect("create temp secret directory");
    let runtime = DesktopPlatformRuntime::with_data_dir(&root);

    assert!(runtime
        .write_secret_if_absent("identity.key", b"secret")
        .expect("create secret"));
    assert!(!runtime
        .write_secret_if_absent("identity.key", b"replacement")
        .expect("refuse replacement"));

    let metadata = std::fs::metadata(root.join("identity.key")).expect("secret metadata");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        runtime.read_secret("identity.key").expect("read secret"),
        Some(b"secret".to_vec())
    );

    std::fs::remove_dir_all(root).expect("remove temp secret directory");
}
