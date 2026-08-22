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
        "qa/tests/api",
        "qa/tests/config",
        "qa/tests/discovery",
        "qa/tests/hygiene",
        "qa/tests/observability",
        "qa/tests/operator",
        "qa/tests/relay",
        "qa/tests/runtime",
        "qa/tests/security",
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
        "active roadmap should live at docs/roadmap.md"
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
        root.join("run-full-validation.cmd").is_file()
            && root.join("run-full-validation.sh").is_file(),
        "canonical Windows/Linux validation launchers must live at the repository root"
    );
    assert!(
        !root.join("qa/ci/run-full-validation.ps1").exists()
            && !root.join("qa/ci/run-full-validation.sh").exists(),
        "legacy qa/ci validation launchers must not return"
    );

    for launcher in ["run-full-validation.cmd", "run-full-validation.sh"] {
        let contents = fs::read_to_string(root.join(launcher)).expect("validation launcher");
        assert!(
            contents.contains("cargo metadata --locked --format-version 1 --no-deps"),
            "{launcher} must verify the committed Cargo.lock without regenerating it"
        );
        assert!(
            !contents.contains("cargo generate-lockfile") && !contents.contains("rm -f Cargo.lock"),
            "{launcher} must never rewrite the dependency graph during production validation"
        );
        assert!(
            contents.contains("cargo-audit")
                && contents.contains("0.22.2")
                && contents.contains("cargo-deny")
                && contents.contains("0.20.2"),
            "{launcher} must pin the validated audit/deny tool versions"
        );
        assert!(
            contents.contains("cargo deny check --config qa/ci/deny.toml --help")
                && contents.contains("cargo deny check --config qa/ci/deny.toml")
                && contents.contains("cargo deny --config qa/ci/deny.toml check"),
            "{launcher} must support both cargo-deny config-option placements"
        );
    }

    let windows = fs::read_to_string(root.join("run-full-validation.cmd"))
        .expect("Windows validation launcher");
    assert!(
        !windows.to_ascii_lowercase().contains("call :"),
        "Windows validation must not rely on CALL :label subroutines"
    );

    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("GitHub Actions workflow");
    assert!(
        workflow.contains("bash ./run-full-validation.sh --skip-ignored --no-pause"),
        "GitHub Actions Unix validation must invoke the root launcher"
    );
    assert!(
        workflow.contains(".\\run-full-validation.cmd --skip-ignored --no-pause"),
        "GitHub Actions Windows validation must invoke the root CMD launcher"
    );
    assert!(
        !workflow.contains("qa/ci/run-full-validation.sh")
            && !workflow.contains("qa\\ci\\run-full-validation.ps1"),
        "GitHub Actions must not reference removed qa/ci validation launchers"
    );
    assert!(
        workflow.contains("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1")
            && !workflow.contains("actions/checkout@v"),
        "GitHub Actions checkout must be pinned to the immutable v7.0.1 commit"
    );
    assert!(
        workflow.contains("rustup toolchain install 1.98.0")
            && !workflow.contains("dtolnay/rust-toolchain")
            && !workflow.contains("Swatinem/rust-cache"),
        "GitHub Actions must use the exact Rust release without floating third-party setup/cache actions"
    );
    assert!(
        workflow.contains("persist-credentials: false") && workflow.contains("contents: read"),
        "GitHub Actions must use least-privilege checkout credentials"
    );

    let toolchain = fs::read_to_string(root.join("rust-toolchain.toml")).expect("toolchain pin");
    assert!(
        toolchain.contains("channel = \"1.98.0\""),
        "rust-toolchain.toml must pin the exact production Rust release"
    );
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml");
    let lock = fs::read_to_string(root.join("Cargo.lock")).expect("committed Cargo.lock");
    assert!(
        !manifest.contains("ratatui") && !lock.contains("name = \"ratatui\"") && !lock.contains("name = \"lru\""),
        "the dashboard must not reintroduce the known-unsound lru dependency through ratatui"
    );
    assert!(
        locked_package_version(&lock, "event-listener") >= Some((5, 4, 2)),
        "event-listener must stay at or above 5.4.2 to exclude RUSTSEC-2026-0221"
    );

    let nightly = fs::read_to_string(root.join(".github/workflows/security-nightly.yml"))
        .expect("nightly security workflow");
    for target in [
        "heartbeat_wire",
        "node_config_json",
        "peer_cache_json",
        "app_message",
        "dnsaddr_txt",
        "peer_multiaddr",
        "webrtc_stun",
    ] {
        assert!(nightly.contains(target), "nightly fuzz workflow must run {target}");
    }
    assert!(
        nightly.contains("nightly-2026-08-20") && nightly.contains("cargo-fuzz --version 0.13.2"),
        "nightly fuzzing must use pinned toolchain/tool versions"
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
    let node_snapshot =
        fs::read_to_string(root.join("crates/node/snapshot.rs")).expect("node snapshot");

    assert!(
        node_snapshot.contains("serde::Serialize"),
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
fn node_config_snapshot_and_validation_live_in_focused_modules() {
    let root = manifest_dir();
    let node_mod = fs::read_to_string(root.join("crates/node/mod.rs")).expect("node mod");
    let node_config = fs::read_to_string(root.join("crates/node/config.rs")).expect("node config");
    let node_validation =
        fs::read_to_string(root.join("crates/node/config_validation.rs")).expect("node validation");
    let node_snapshot =
        fs::read_to_string(root.join("crates/node/snapshot.rs")).expect("node snapshot");

    assert!(
        node_mod.contains("mod config;")
            && node_mod.contains("mod config_validation;")
            && node_mod.contains("mod snapshot;"),
        "node mod should declare focused config, validation, and snapshot modules"
    );
    assert!(
        !root.join("crates/node/types.rs").exists(),
        "mixed node types file should be removed after the split"
    );
    assert!(
        node_config.contains("pub struct NodeConfig")
            && !node_config.contains("pub struct NodeSnapshot"),
        "config.rs should own NodeConfig only"
    );
    assert!(
        node_snapshot.contains("pub struct NodeSnapshot")
            && !node_snapshot.contains("pub struct NodeConfig"),
        "snapshot.rs should own NodeSnapshot only"
    );
    assert!(
        node_validation.contains("pub(crate) fn validate_node_config")
            && node_validation.contains("pub(crate) fn parse_multiaddrs"),
        "config validation and parsing helpers belong in config_validation.rs"
    );
}

fn locked_package_version(lock: &str, package: &str) -> Option<(u64, u64, u64)> {
    let name_line = format!("name = \"{package}\"");
    let section = lock
        .split("[[package]]")
        .find(|section| section.lines().any(|line| line == name_line))?;
    let version = section
        .lines()
        .find_map(|line| line.strip_prefix("version = \""))?
        .strip_suffix('"')?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split('-').next()?.parse().ok()?;
    Some((major, minor, patch))
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
