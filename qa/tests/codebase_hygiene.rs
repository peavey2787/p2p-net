use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn runtime_docs_do_not_contain_transitional_phase_language() {
    let root = manifest_dir();
    let scan_roots = ["crates", "docs", "qa/ci", "README.md", "Cargo.toml"];
    let forbidden = [
        "Phase 1",
        "Phase 2",
        "Phase 3",
        "Phase 4",
        "Phase 5",
        "Phase 6",
        "Phase 7",
        "Phase 8",
        "Phase 9",
        "Phase 10",
        "later phase",
        "later phases",
        "legacy behavior",
        "legacy `",
        "current single-crate",
        "compatibility adapter",
        "all-in-one",
    ];

    let mut violations = Vec::new();
    for path in text_files_under(&root, &scan_roots) {
        let rel = path.strip_prefix(&root).unwrap_or(&path);
        if rel == Path::new("docs/roadmap.md") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read text file");
        for pattern in forbidden {
            if text.contains(pattern) {
                violations.push(format!("{} contains `{pattern}`", rel.display()));
            }
        }
    }

    assert!(violations.is_empty(), "{}", violations.join("\n"));
}


#[test]
fn repository_layout_matches_modular_baseline() {
    let root = manifest_dir();
    for dir in [
        "crates",
        "docs/impl",
        "docs/spec",
        "docs/validation",
        "docs/project",
        "docs/future-work",
        "qa/ci",
        "qa/fuzz",
        "qa/tools",
        "qa/vectors",
        "qa/tests",
        "examples",
        "external",
    ] {
        assert!(root.join(dir).is_dir(), "missing expected directory: {dir}");
    }

    for old_dir in ["src", "unit_tests", "scripts", "fuzz", "vendor"] {
        assert!(
            !root.join(old_dir).exists(),
            "old top-level directory should not remain: {old_dir}"
        );
    }

    assert!(
        root.join("docs/roadmap.md").is_file(),
        "active discovery roadmap should live at docs/roadmap.md"
    );
    assert!(
        !root.join(Path::new("crates").join("p2p-net")).exists(),
        "runtime code must live directly under crates/, without an extra crate-name nesting directory"
    );
    assert!(
        !root.join(Path::new("external").join("vendor")).exists(),
        "local third-party patches must live directly under external/, without an extra vendor nesting directory"
    );
    assert!(
        root.join("qa/ci/run-full-validation.ps1").is_file()
            && root.join("qa/ci/run-full-validation.sh").is_file(),
        "canonical validation scripts belong under qa/ci/"
    );
}

#[test]
fn scripts_do_not_use_phase_specific_tooling_names() {
    let root = manifest_dir();
    let scripts_dir = root.join("qa/ci");
    let mut violations = Vec::new();

    for entry in fs::read_dir(&scripts_dir).expect("scripts dir") {
        let entry = entry.expect("script entry");
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("script file name");
        let lower_name = file_name.to_ascii_lowercase();
        if lower_name.contains("phase")
            || lower_name.contains("legacy")
            || lower_name.contains("deprecated")
        {
            violations.push(file_name.to_string());
        }
    }

    assert!(
        violations.is_empty(),
        "phase-specific or legacy script names should not remain: {violations:?}"
    );
}

#[test]
fn profile_decisions_are_not_duplicated_in_startup_or_stack_layers() {
    let root = manifest_dir();
    let node_mod = fs::read_to_string(root.join("crates/node/mod.rs")).expect("node mod");
    let behaviour = fs::read_to_string(root.join("crates/stack/behaviour.rs")).expect("behaviour");
    let transport = fs::read_to_string(root.join("crates/stack/transport.rs")).expect("transport");

    assert!(
        node_mod.contains("try_resolved_for_environment"),
        "startup must resolve capabilities through the central resolver"
    );
    assert!(
        node_mod.contains("build_swarm(local_key, &cfg, &resolved_config)"),
        "startup must pass resolved capability policy into the stack builder"
    );
    assert!(
        !node_mod.contains("match cfg.profile"),
        "startup must not duplicate profile-to-role decisions"
    );
    assert!(
        !behaviour.contains("NodeProfile") && !transport.contains("NodeProfile"),
        "stack builders must consume resolved behaviour policy, not user-facing profiles"
    );
}

#[test]
fn snapshot_json_uses_derived_serialization_instead_of_duplicate_field_mapping() {
    let root = manifest_dir();
    let node_mod = fs::read_to_string(root.join("crates/node/mod.rs")).expect("node mod");
    let node_types = fs::read_to_string(root.join("crates/node/types.rs")).expect("node types");

    assert!(
        node_types.contains("serde::Serialize"),
        "NodeSnapshot should derive serialization so JSON output cannot drift from snapshot fields"
    );
    assert!(
        node_mod.contains("serde_json::to_value(snapshot)"),
        "snapshot_to_json should serialize NodeSnapshot directly"
    );
    assert!(
        !node_mod.contains("fn insert<T: serde::Serialize>"),
        "snapshot_to_json must not duplicate the NodeSnapshot field list by hand"
    );
}

#[test]
fn node_prometheus_metrics_live_in_metrics_module() {
    let root = manifest_dir();
    let node_mod = fs::read_to_string(root.join("crates/node/mod.rs")).expect("node mod");
    let node_metrics =
        fs::read_to_string(root.join("crates/node/metrics.rs")).expect("node metrics");

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
        node_metrics.contains("pub fn snapshot_to_prometheus_metrics"),
        "snapshot-to-Prometheus rendering belongs in crates/node/metrics.rs"
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
        cargo.contains("external/libp2p-dns")
            && cargo.contains("external/libp2p-mdns-placeholder"),
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

    assert!(duplicate_names.is_empty(), "duplicate test names: {duplicate_names:?}");
    assert!(duplicate_paths.is_empty(), "duplicate test paths: {duplicate_paths:?}");

    for entry in fs::read_dir(root.join("qa/tests")).expect("qa/tests dir") {
        let entry = entry.expect("unit test entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        assert!(paths.contains(rel.as_str()), "unregistered integration test: {rel}");
    }
}

fn text_files_under(root: &Path, entries: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in entries {
        let path = root.join(entry);
        if path.is_file() {
            out.push(path);
        } else if path.is_dir() {
            collect_text_files(&path, &mut out);
        }
    }
    out
}

fn collect_text_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_text_files(&path, out);
        } else if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("rs" | "md" | "toml")
        ) {
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
