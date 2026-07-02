use std::fs;
use std::path::Path;

use p2p_net::{NodeConfig, NodeProfile, NodeRole};

#[test]
fn lite_profiles_resolve_to_kademlia_client_only() {
    for profile in [NodeProfile::Lite, NodeProfile::MobileLite] {
        let cfg = NodeConfig {
            profile,
            ..NodeConfig::default()
        };
        let resolved = cfg.try_resolved().expect("profile resolves");

        assert!(matches!(resolved.role, NodeRole::Lite | NodeRole::MobileLite));
        assert!(resolved.enabled_behaviours.kademlia_client);
        assert!(!resolved.enabled_behaviours.kademlia_server);
        assert!(resolved.enabled_behaviours.relay_client);
        assert!(resolved.enabled_behaviours.dcutr);
        assert!(!resolved.enabled_behaviours.relay_server);
        assert!(!resolved.enabled_behaviours.rendezvous_server);
    }
}

#[test]
fn infrastructure_profiles_keep_kademlia_server_mode() {
    for profile in [
        NodeProfile::Full,
        NodeProfile::Relay,
        NodeProfile::Mediator,
        NodeProfile::Rendezvous,
        NodeProfile::Bootstrap,
    ] {
        let cfg = NodeConfig {
            profile,
            ..NodeConfig::default()
        };
        let resolved = cfg.try_resolved().expect("profile resolves");

        assert!(resolved.enabled_behaviours.kademlia_server);
    }
}

#[test]
fn behaviour_builder_uses_resolved_policy_instead_of_hard_coded_server_mode() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let behaviour_rs = fs::read_to_string(manifest_dir.join("crates/stack/behaviour.rs"))
        .expect("read behaviour source");
    let transport_rs = fs::read_to_string(manifest_dir.join("crates/stack/transport.rs"))
        .expect("read transport source");

    assert!(behaviour_rs.contains("pub resolved_cfg: &'a ResolvedNodeConfig"));
    assert!(behaviour_rs.contains("build_behaviour(ctx: BehaviourBuildContext<'_>)"));
    assert!(behaviour_rs.contains("kad::Mode::Client"));
    assert!(!behaviour_rs.contains("kademlia.set_mode(Some(kad::Mode::Server));"));
    assert!(transport_rs.contains("resolved_cfg: &ResolvedNodeConfig"));
    assert!(transport_rs.contains("kademlia-client"));
    assert!(transport_rs.contains("kademlia-server"));
}
