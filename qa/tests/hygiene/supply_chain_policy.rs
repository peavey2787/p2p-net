//! Supply-chain policy: exact advisory exceptions, manifest-backed security
//! floors, and retired advisories that must not return.

use std::fs;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn advisory_exceptions_and_security_floors_are_exact() {
    let root = manifest_dir();
    let audit_config =
        fs::read_to_string(root.join("qa/ci/audit.toml")).expect("cargo-audit config");
    for required in [
        "ignore = [\"RUSTSEC-2024-0436\"]",
        "deny = [\"unmaintained\", \"unsound\"]",
        "format = \"terminal\"",
        "quiet = false",
        "show_tree = true",
    ] {
        assert!(
            audit_config.contains(required),
            "qa/ci/audit.toml must keep the exact temporary unmaintained-debt policy and cargo-audit 0.22.2 output fields: {required}"
        );
    }
    let deny_config = fs::read_to_string(root.join("qa/ci/deny.toml")).expect("cargo-deny config");
    for required in [
        "unmaintained = \"all\"",
        "unsound = \"all\"",
        "unused-ignored-advisory = \"deny\"",
        "id = \"RUSTSEC-2024-0436\"",
    ] {
        assert!(
            deny_config.contains(required),
            "qa/ci/deny.toml must reject new unmaintained/unsound advisories and keep only tracked temporary debt: {required}"
        );
    }
    assert_eq!(
        deny_config.matches("id = \"RUSTSEC-").count(),
        1,
        "cargo-deny may temporarily ignore only the one documented unmaintained transitive advisory"
    );

    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml");
    let lock = fs::read_to_string(root.join("Cargo.lock")).expect("committed Cargo.lock");
    assert!(
        !manifest.contains("ratatui")
            && !lock.contains("name = \"ratatui\"")
            && !lock.contains("name = \"lru\""),
        "the dashboard must not reintroduce the known-unsound lru dependency through ratatui"
    );
    assert!(
        manifest.contains("event-listener = \">=5.4.2, <6\""),
        "Cargo.toml must keep an explicit event-listener security floor so the committed lockfile has a manifest-backed patched resolution"
    );
    assert!(
        locked_package_version(&lock, "event-listener") >= Some((5, 4, 2)),
        "event-listener must stay at or above 5.4.2 to exclude RUSTSEC-2026-0221"
    );
    assert!(
        manifest.contains("h2 = \">=0.4.16, <0.5\""),
        "Cargo.toml must keep an explicit h2 security floor so lock regeneration cannot reintroduce RUSTSEC-2026-0258"
    );
    assert!(
        locked_package_version(&lock, "h2") >= Some((0, 4, 16)),
        "h2 must stay at or above 0.4.16 to exclude RUSTSEC-2026-0258"
    );
    assert_eq!(
        locked_package_version(&lock, "bincode"),
        None,
        "bincode (RUSTSEC-2025-0141) left the graph with the WebRTC 0.17 migration and must not return"
    );
    assert!(
        !audit_config.contains("RUSTSEC-2025-0141") && !deny_config.contains("RUSTSEC-2025-0141"),
        "the retired bincode advisory exception must stay removed from both advisory lists"
    );
    assert!(
        manifest.contains("rkyv = { version = \">=0.8.18, <0.9\""),
        "Cargo.toml must keep the manifest-backed rkyv security floor for the WebRTC 0.17 stack"
    );
    assert!(
        locked_package_version(&lock, "rkyv") >= Some((0, 8, 18)),
        "rkyv must stay at or above 0.8.18"
    );
    assert_eq!(
        locked_package_version(&lock, "paste"),
        Some((1, 0, 15)),
        "when paste leaves the graph, remove RUSTSEC-2024-0436 from both advisory exception lists in the same change"
    );

    let monthly = fs::read_to_string(root.join(".github/workflows/security-monthly.yml"))
        .expect("monthly security workflow");
    assert!(
        monthly.contains("name: p2p-net-security-monthly")
            && monthly.contains("cron: '23 4 1 * *'")
            && monthly.contains("workflow_dispatch:"),
        "heavy security validation must run once per month and remain manually dispatchable"
    );
    assert!(
        !monthly.contains("cron: '23 4 * * *'"),
        "heavy security validation must not regress to a nightly schedule"
    );
    for target in [
        "heartbeat_wire",
        "node_config_json",
        "peer_cache_json",
        "app_message",
        "dnsaddr_txt",
        "peer_multiaddr",
        "webrtc_stun",
    ] {
        assert!(
            monthly.contains(target),
            "monthly fuzz workflow must run {target}"
        );
    }
    assert!(
        monthly.contains("nightly-2026-08-20") && monthly.contains("cargo-fuzz --version 0.13.2"),
        "monthly fuzzing must use pinned toolchain/tool versions"
    );
    assert!(
        monthly.contains("fuzz build --fuzz-dir qa/fuzz")
            && monthly.contains("fuzz run --fuzz-dir qa/fuzz"),
        "monthly fuzzing must explicitly select the nested qa/fuzz harness"
    );
    assert!(
        !monthly.contains("working-directory: qa/fuzz"),
        "working-directory alone is insufficient for cargo-fuzz nested-harness discovery; use --fuzz-dir qa/fuzz"
    );
    assert!(
        monthly.contains("test -f qa/fuzz/Cargo.toml"),
        "monthly fuzzing must fail early if the expected fuzz manifest is missing"
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
