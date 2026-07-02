use std::fs;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn original_srp_hotspots_stay_split() {
    let root = manifest_dir();

    assert!(
        !root.join("crates/node/types.rs").exists(),
        "mixed node types file must not return"
    );
    assert!(
        lines(&root.join("crates/node/mod.rs")) <= 450,
        "node orchestration should stay below the SRP threshold"
    );
    assert!(
        lines(&root.join("crates/connectivity/relay.rs")) <= 80,
        "relay.rs should remain a small facade over focused relay modules"
    );

    for (path, max_lines) in [
        ("crates/node/config.rs", 280),
        ("crates/node/config_validation.rs", 220),
        ("crates/node/metrics.rs", 260),
        ("crates/node/runtime.rs", 320),
        ("crates/node/snapshot.rs", 240),
        ("crates/node/startup.rs", 280),
        ("crates/connectivity/relay/address.rs", 100),
        ("crates/connectivity/relay/config.rs", 320),
        ("crates/connectivity/relay/schedule.rs", 220),
        ("crates/connectivity/relay/state.rs", 140),
    ] {
        assert!(
            lines(&root.join(path)) <= max_lines,
            "{path} exceeded focused-module line budget of {max_lines} lines"
        );
    }
}

#[test]
fn longest_source_files_stay_under_hygiene_budget() {
    let root = manifest_dir();
    let (crate_path, crate_lines) = largest_rust_file_under(&root.join("crates"));
    assert!(
        crate_lines <= 500,
        "runtime source file too large: {} has {crate_lines} lines",
        crate_path.display()
    );

    let (qa_path, qa_lines) = largest_rust_file_under(&root.join("qa/tests"));
    assert!(
        qa_lines <= 500,
        "QA source file too large: {} has {qa_lines} lines",
        qa_path.display()
    );
}

#[test]
fn duplicate_cleanup_helpers_do_not_return() {
    let root = manifest_dir();
    assert_single_definition(
        &root,
        "fn config_error(",
        "crates/common/error.rs",
        "configuration error helper must stay centralized",
    );
    assert_single_definition(
        &root,
        "fn has_reachable_transport(",
        "crates/connectivity/addr.rs",
        "reachable transport helper must stay centralized",
    );
    assert_single_definition(
        &root,
        "fn has_unspecified_ip(",
        "crates/connectivity/addr.rs",
        "unspecified-IP helper must stay centralized",
    );
}

#[test]
fn final_audit_document_matches_current_layout() {
    let root = manifest_dir();
    let audit = fs::read_to_string(root.join("docs/project/FINAL_HYGIENE_AUDIT.md"))
        .expect("final hygiene audit doc");
    let roadmap = fs::read_to_string(root.join("docs/roadmap.md")).expect("roadmap doc");

    for required in [
        "crates/connectivity/dns.rs — 471 lines",
        "crates/node/mod.rs — 421 lines",
        "crates/connectivity/relay.rs — 13 lines",
        "`qa/tests/` is domain-grouped",
        "Step 9 — Final hygiene audit",
    ] {
        assert!(
            audit.contains(required) || roadmap.contains(required),
            "final audit docs should mention `{required}`"
        );
    }
}

fn lines(path: &Path) -> usize {
    fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
        .lines()
        .count()
}

fn largest_rust_file_under(root: &Path) -> (PathBuf, usize) {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files
        .into_iter()
        .map(|path| {
            let line_count = lines(&path);
            (path, line_count)
        })
        .max_by_key(|(_, line_count)| *line_count)
        .expect("at least one Rust file")
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display())) {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn assert_single_definition(root: &Path, needle: &str, expected_rel_path: &str, message: &str) {
    let mut matches = Vec::new();
    for path in text_files_under(&root.join("crates")) {
        let text = fs::read_to_string(&path).expect("read source file");
        if text.contains(needle) {
            matches.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }

    assert_eq!(
        matches,
        vec![expected_rel_path.to_string()],
        "{message}"
    );
}

fn text_files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_text_files(root, &mut out);
    out
}

fn collect_text_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display())) {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_text_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}
