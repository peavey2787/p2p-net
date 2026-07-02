use std::fs;
use std::path::PathBuf;

use p2p_net::NodeConfig;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn operator_guides_are_present_and_link_examples() {
    let root = root();
    let overview = fs::read_to_string(root.join("docs/operator/README.md")).expect("operator guide");
    let private = fs::read_to_string(root.join("docs/operator/PRIVATE_INFRASTRUCTURE_FIRST.md"))
        .expect("private infrastructure guide");
    let public = fs::read_to_string(root.join("docs/operator/PUBLIC_FALLBACK_TRADEOFFS.md"))
        .expect("public fallback guide");
    let fleet = fs::read_to_string(root.join("docs/operator/PRODUCTION_FLEET.md"))
        .expect("production fleet guide");

    assert!(overview.contains("PRIVATE_INFRASTRUCTURE_FIRST.md"));
    assert!(overview.contains("PUBLIC_FALLBACK_TRADEOFFS.md"));
    assert!(overview.contains("PRODUCTION_FLEET.md"));
    assert!(private.contains("examples/private-infrastructure-first.config.json"));
    assert!(private.contains("public_bootstrap.mode` is `disabled"));
    assert!(public.contains("examples/public-fallback.config.json"));
    assert!(public.contains("fallback_only"));
    assert!(public.contains("Public fallback is the normal app default"));
    for required_role in ["Bootstrap", "Rendezvous", "Mediator", "Relay"] {
        assert!(fleet.contains(required_role), "missing role {required_role}");
    }
}

#[test]
fn operator_example_configs_validate() {
    let private = load_config("examples/private-infrastructure-first.config.json");
    private.validate().expect("private infra config validates");
    assert_eq!(private.discovery.public_bootstrap.mode.as_str(), "disabled");
    assert!(private.discovery.public_bootstrap.bootstrap_seed_peers.is_empty());
    assert!(private.discovery.public_bootstrap.rendezvous_peers.is_empty());
    assert!(!private.discovery.public_bootstrap.auto_connect_discovered_peers);
    assert_eq!(private.discovery.namespace.privacy.as_str(), "hashed");
    assert!(!private.discovery.namespace.allow_readable_tags);
    assert!(private.discovery.rendezvous.client_enabled);
    assert!(!private.discovery.rendezvous.server_enabled);

    let public = load_config("examples/public-fallback.config.json");
    public.validate().expect("public fallback config validates");
    assert_eq!(public.discovery.public_bootstrap.mode.as_str(), "fallback_only");
    assert!(public.discovery.public_bootstrap.bootstrap_seed_peers.len() >= 4);
    assert!(public.discovery.public_bootstrap.rendezvous_peers.is_empty());
    assert!(public.discovery.public_bootstrap.relay_peers.is_empty());
    assert!(public.discovery.public_bootstrap.auto_connect_discovered_peers);
    assert_eq!(public.discovery.namespace.privacy.as_str(), "hashed");
    assert!(!public.discovery.namespace.allow_readable_tags);
}

fn load_config(path: &str) -> NodeConfig {
    let raw = fs::read_to_string(root().join(path)).expect("read example config");
    serde_json::from_str(&raw).expect("parse example config")
}
