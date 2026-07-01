use p2p_net::{
    binding_support_matrix, node_config_from_json, node_config_to_json,
    node_snapshot_to_json_string, prepare_binding_start_plan, BindingRuntimeSpec,
    BindingStorageRequirement, BindingStorageStrategy, BindingTarget, NetworkReachability,
    NodeConfig, NodeRole, NodeSnapshot, PlatformKind, PlatformRuntime,
};

#[test]
fn binding_matrix_keeps_one_shared_core_for_all_shells() {
    let matrix = binding_support_matrix();

    assert!(matrix.shared_rust_core);
    assert!(!matrix.separate_node_implementations_required);
    assert_eq!(matrix.targets.len(), 4);
    assert!(matrix
        .targets
        .iter()
        .any(|target| target.target == BindingTarget::Android
            && target.storage_requirement == BindingStorageRequirement::ExternalPlatformStorage
            && target.background_limited));
    assert!(matrix
        .targets
        .iter()
        .any(|target| target.target == BindingTarget::Desktop
            && target.storage_requirement == BindingStorageRequirement::DesktopFilesystem
            && !target.background_limited));
}

#[test]
fn binding_runtime_spec_resolves_android_auto_to_mobile_lite() {
    let cfg = NodeConfig::default();
    let config_json = node_config_to_json(&cfg).expect("serialize config");
    let mut spec = BindingRuntimeSpec::android(Some("/data/user/0/app/p2p-net".to_string()));
    spec.storage = BindingStorageStrategy::ExternalPlatformStorage;

    let plan = prepare_binding_start_plan(&config_json, &spec).expect("prepare android plan");

    assert_eq!(plan.target, BindingTarget::Android);
    assert_eq!(plan.platform, PlatformKind::Android);
    assert_eq!(plan.resolved_role, NodeRole::MobileLite);
    assert!(!plan.should_listen);
    assert!(plan.enabled_behaviours.relay_client);
    assert!(plan.enabled_behaviours.dcutr);
    assert!(plan.dcutr_enabled);
    assert_eq!(
        plan.storage_requirement,
        BindingStorageRequirement::ExternalPlatformStorage
    );
    assert_eq!(plan.environment.reachability, NetworkReachability::PrivateNat);
    assert!(plan
        .warnings
        .iter()
        .any(|warning| warning.contains("durable NodeStorage")));
}

#[test]
fn binding_runtime_spec_preserves_desktop_inbound_defaults() {
    let spec = BindingRuntimeSpec::desktop(Some("C:/Users/example/AppData/p2p-net".to_string()));
    let runtime = spec.runtime();

    assert_eq!(runtime.runtime_name(), "binding_desktop");
    assert_eq!(runtime.platform_kind(), PlatformKind::current());
    assert!(runtime.can_listen_tcp());
    assert!(runtime.can_listen_quic());
    assert!(runtime.can_accept_inbound().is_none());
    assert!(!runtime.is_background_restricted());
    assert_eq!(
        spec.storage_requirement(),
        BindingStorageRequirement::DesktopFilesystem
    );
}

#[test]
fn binding_json_helpers_validate_config_and_snapshot() {
    let cfg = NodeConfig::default();
    let json = node_config_to_json(&cfg).expect("serialize node config");
    let parsed = node_config_from_json(&json).expect("parse binding config");

    assert_eq!(parsed.network_id, cfg.network_id);
    assert!(node_config_from_json("{ not json }").is_err());

    let snapshot_json = node_snapshot_to_json_string(&NodeSnapshot {
        peer_id: "test-peer".to_string(),
        platform_runtime: "binding_android".to_string(),
        platform_storage: "external".to_string(),
        ..NodeSnapshot::default()
    })
    .expect("serialize snapshot for binding UI");

    assert!(snapshot_json.contains("binding_android"));
    assert!(snapshot_json.contains("test-peer"));
}

#[test]
fn memory_storage_warns_as_testing_only_for_bindings() {
    let cfg = NodeConfig::default();
    let config_json = node_config_to_json(&cfg).expect("serialize config");
    let mut spec = BindingRuntimeSpec::ios(Some("/app/group/p2p-net".to_string()));
    spec.storage = BindingStorageStrategy::MemoryTestingOnly;

    let plan = prepare_binding_start_plan(&config_json, &spec).expect("prepare ios plan");

    assert_eq!(plan.target, BindingTarget::Ios);
    assert_eq!(plan.resolved_role, NodeRole::MobileLite);
    assert_eq!(
        plan.storage_requirement,
        BindingStorageRequirement::MemoryTestingOnly
    );
    assert!(plan
        .warnings
        .iter()
        .any(|warning| warning.contains("ephemeral")));
}
