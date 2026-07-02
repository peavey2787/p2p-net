use p2p_net::{
    build_discovery_namespace, discovery_tag_hash_hex, DiscoveryConfig, DiscoveryNamespaceConfig,
    DiscoveryNamespacePrivacy, NodeConfig, RendezvousConfig,
};

#[test]
fn hashed_discovery_namespace_does_not_publish_raw_tag() {
    let raw_tag = "IAmJoeTheRealJoeYouWant213423432";
    let namespace = build_discovery_namespace(
        1,
        "hydra-msg",
        raw_tag,
        DiscoveryNamespacePrivacy::Hashed,
        false,
    )
    .expect("hashed namespace");

    assert!(namespace.namespace.starts_with("p2p-net/1/hydra-msg/"));
    assert!(!namespace.namespace.contains(raw_tag));
    assert_eq!(namespace.tag_fingerprint_hex.len(), 64);
    assert_eq!(
        namespace.tag_fingerprint_hex,
        discovery_tag_hash_hex(1, "hydra-msg", raw_tag)
    );
}

#[test]
fn readable_discovery_namespace_requires_debug_guardrail() {
    let cfg = DiscoveryConfig {
        namespace: DiscoveryNamespaceConfig {
            app_id: "hydra-msg".to_string(),
            tags: vec!["Joe".to_string()],
            privacy: DiscoveryNamespacePrivacy::ReadableUnsafe,
            allow_readable_tags: false,
        },
        ..DiscoveryConfig::default()
    };
    assert!(cfg.validate().is_err());

    let cfg = DiscoveryConfig {
        namespace: DiscoveryNamespaceConfig {
            app_id: "hydra-msg".to_string(),
            tags: vec!["Joe".to_string()],
            privacy: DiscoveryNamespacePrivacy::ReadableUnsafe,
            allow_readable_tags: true,
        },
        ..DiscoveryConfig::default()
    };
    cfg.validate().expect("explicit debug-readable config validates");
    let namespaces = cfg.rendezvous_namespaces(1).expect("namespaces");
    assert_eq!(namespaces, vec!["p2p-net/1/hydra-msg/joe".to_string()]);
}

#[test]
fn multiple_tags_derive_multiple_unique_namespaces() {
    let cfg = DiscoveryConfig {
        namespace: DiscoveryNamespaceConfig {
            app_id: "hydra-msg".to_string(),
            tags: vec![
                "contact-a".to_string(),
                "contact-b".to_string(),
                "contact-a".to_string(),
            ],
            ..DiscoveryNamespaceConfig::default()
        },
        ..DiscoveryConfig::default()
    };

    let namespaces = cfg.rendezvous_namespaces(42).expect("namespaces");
    assert_eq!(namespaces.len(), 2);
    assert!(namespaces.iter().all(|ns| ns.starts_with("p2p-net/42/hydra-msg/")));
}

#[test]
fn empty_app_tags_preserve_operator_rendezvous_namespace() {
    let cfg = DiscoveryConfig {
        rendezvous: RendezvousConfig {
            namespace: "operator-owned-mesh".to_string(),
            ..RendezvousConfig::default()
        },
        ..DiscoveryConfig::default()
    };

    assert_eq!(
        cfg.rendezvous_namespaces(7).expect("namespaces"),
        vec!["operator-owned-mesh".to_string()]
    );
}

#[test]
fn node_config_validates_discovery_namespace_config() {
    let bad = NodeConfig {
        discovery: DiscoveryConfig {
            namespace: DiscoveryNamespaceConfig {
                app_id: "hydra-msg".to_string(),
                tags: vec!["".to_string()],
                ..DiscoveryNamespaceConfig::default()
            },
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    assert!(bad.validate().is_err());

    let good = NodeConfig {
        discovery: DiscoveryConfig {
            namespace: DiscoveryNamespaceConfig {
                app_id: "hydra-msg".to_string(),
                tags: vec!["Joe invite tag".to_string()],
                ..DiscoveryNamespaceConfig::default()
            },
            ..DiscoveryConfig::default()
        },
        ..NodeConfig::default()
    };
    good.validate().expect("node config validates");
}
