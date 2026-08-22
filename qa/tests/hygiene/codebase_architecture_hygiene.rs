use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn node_prometheus_metrics_live_in_metrics_module() {
    let root = manifest_dir();
    let node_mod = fs::read_to_string(root.join("crates/node/mod.rs")).expect("node mod");
    let node_metrics =
        fs::read_to_string(root.join("crates/node/metrics.rs")).expect("node metrics");
    let prometheus_metrics = fs::read_to_string(root.join("crates/node/metrics/prometheus.rs"))
        .expect("node Prometheus metrics");

    assert!(
        node_mod.contains("mod metrics;"),
        "node mod must declare the focused metrics module"
    );
    assert!(
        node_mod.contains("pub use metrics::snapshot_to_prometheus_metrics;"),
        "public metrics export should be re-exported from the focused module"
    );
    assert!(
        !node_mod.contains("p2p_connected_peers"),
        "node orchestration must not contain Prometheus metric formatting"
    );
    assert!(
        node_metrics.contains("mod prometheus;")
            && node_metrics.contains("pub fn snapshot_to_prometheus_metrics"),
        "crates/node/metrics.rs should stay a small facade over focused metrics implementations"
    );
    assert!(
        prometheus_metrics.contains("p2p_connected_peers")
            && prometheus_metrics.contains("pub(crate) fn snapshot_to_prometheus_metrics"),
        "snapshot-to-Prometheus rendering belongs in crates/node/metrics/prometheus.rs"
    );
}

#[test]
fn node_startup_discovery_setup_lives_in_startup_module() {
    let root = manifest_dir();
    let node_mod = fs::read_to_string(root.join("crates/node/mod.rs")).expect("node mod");
    let node_startup =
        fs::read_to_string(root.join("crates/node/startup.rs")).expect("node startup");
    let startup_addrs = fs::read_to_string(root.join("crates/node/startup/addrs.rs"))
        .expect("startup address module");

    assert!(
        node_mod.contains("mod startup;"),
        "node mod must declare the focused startup module"
    );
    assert!(
        node_mod.contains("prepare_startup_discovery"),
        "node orchestration should delegate startup discovery preparation"
    );
    assert!(
        !node_mod.contains("resolve_configured_multiaddrs"),
        "DNS-backed startup address resolution belongs in crates/node/startup.rs"
    );
    assert!(
        !node_mod.contains("load_last_addrs_with_storage"),
        "cached startup address loading belongs in crates/node/startup.rs"
    );
    assert!(
        node_startup.contains("pub(crate) async fn prepare_startup_discovery"),
        "startup discovery preparation belongs in crates/node/startup.rs"
    );
    assert!(
        startup_addrs.contains("struct StartupAddrs"),
        "resolved startup addresses should be grouped before orchestration consumes them"
    );
}

#[test]
fn node_runtime_loop_lives_in_runtime_module() {
    let root = manifest_dir();
    let node_mod = fs::read_to_string(root.join("crates/node/mod.rs")).expect("node mod");
    let node_runtime =
        fs::read_to_string(root.join("crates/node/runtime.rs")).expect("node runtime");

    assert!(
        node_mod.contains("mod runtime;"),
        "node mod must declare the focused runtime module"
    );
    assert!(
        node_mod.contains("spawn_node_runtime"),
        "node startup should hand long-running execution to the runtime module"
    );
    assert!(
        !node_mod.contains("tokio::select!"),
        "node startup must not own the long-running select loop"
    );
    assert!(
        node_runtime.contains("pub(crate) struct NodeRuntimeContext"),
        "runtime ownership should be grouped in a named context struct"
    );
    assert!(
        node_runtime.contains("struct RuntimeState"),
        "mutable loop state should be grouped instead of kept as loose startup locals"
    );
}

#[test]
fn relay_responsibilities_live_in_focused_modules() {
    let root = manifest_dir();
    let relay_mod =
        fs::read_to_string(root.join("crates/connectivity/relay.rs")).expect("relay mod");
    let relay_config =
        fs::read_to_string(root.join("crates/connectivity/relay/config.rs")).expect("relay config");
    let relay_schedule = fs::read_to_string(root.join("crates/connectivity/relay/schedule.rs"))
        .expect("relay schedule");
    let relay_state =
        fs::read_to_string(root.join("crates/connectivity/relay/state.rs")).expect("relay state");
    let relay_address = fs::read_to_string(root.join("crates/connectivity/relay/address.rs"))
        .expect("relay address");

    assert!(
        relay_mod.contains("mod address;")
            && relay_mod.contains("mod config;")
            && relay_mod.contains("mod schedule;")
            && relay_mod.contains("mod state;"),
        "relay facade should declare focused relay modules"
    );
    assert!(
        relay_config.contains("pub struct RelayServiceConfig")
            && relay_config.contains("pub enum RelayAccess")
            && !relay_config.contains("pub struct RelayState"),
        "relay/config.rs should own relay service configuration only"
    );
    assert!(
        relay_schedule.contains("pub struct RelaySchedule")
            && relay_schedule.contains("pub struct RelayWindow")
            && !relay_schedule.contains("pub struct RelayServiceConfig"),
        "relay/schedule.rs should own relay scheduling only"
    );
    assert!(
        relay_state.contains("pub struct RelayState")
            && relay_state.contains("pub struct RelayReservationPlan")
            && relay_state.contains("pub enum RelayServiceHealth")
            && !relay_state.contains("pub struct RelayServiceConfig"),
        "relay/state.rs should own relay runtime state only"
    );
    assert!(
        relay_address.contains("pub fn relay_reservation_addr")
            && relay_address.contains("pub fn is_p2p_circuit_addr")
            && relay_address.contains("pub fn relay_peer_id")
            && !relay_address.contains("pub struct RelayServiceConfig"),
        "relay/address.rs should own relay multiaddr helpers only"
    );
}

#[test]
fn cargo_test_registrations_are_unique_and_complete() {
    let root = manifest_dir();
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml");
    assert!(
        cargo.contains("path = \"crates/lib.rs\""),
        "library source must live under crates"
    );
    assert!(
        cargo.contains("path = \"qa/tests/"),
        "integration tests must be registered from qa/tests"
    );
    assert!(
        cargo.contains("external/libp2p-dns") && cargo.contains("external/libp2p-mdns-placeholder"),
        "local third-party patches must live under external"
    );

    let mut names = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut duplicate_names = Vec::new();
    let mut duplicate_paths = Vec::new();

    for block in cargo.split("[[test]]").skip(1) {
        let name = extract_toml_string(block, "name").expect("test name");
        let path = extract_toml_string(block, "path").expect("test path");
        if !names.insert(name.clone()) {
            duplicate_names.push(name);
        }
        if !paths.insert(path.clone()) {
            duplicate_paths.push(path);
        }
    }

    assert!(
        duplicate_names.is_empty(),
        "duplicate test names: {duplicate_names:?}"
    );
    assert!(
        duplicate_paths.is_empty(),
        "duplicate test paths: {duplicate_paths:?}"
    );

    let mut root_level_tests = Vec::new();
    for entry in fs::read_dir(root.join("qa/tests")).expect("qa/tests dir") {
        let entry = entry.expect("unit test entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            root_level_tests.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert!(
        root_level_tests.is_empty(),
        "integration tests must live under domain folders, not directly under qa/tests/: {root_level_tests:?}"
    );

    let mut test_files = Vec::new();
    collect_rust_files(&root.join("qa/tests"), &mut test_files);
    for path in test_files {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            paths.contains(rel.as_str()),
            "unregistered integration test: {rel}"
        );
    }
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn extract_toml_string(block: &str, key: &str) -> Option<String> {
    block.lines().find_map(|line| {
        let line = line.trim();
        let prefix = format!("{key} = \"");
        line.strip_prefix(&prefix)
            .and_then(|rest| rest.split_once('"'))
            .map(|(value, _)| value.to_string())
    })
}
