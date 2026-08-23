use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap_or_else(|_| panic!("missing {path}"))
}

#[test]
fn canonical_release_runners_verify_two_clean_builds() {
    let root = root();
    let windows_launcher = read(&root, "build-release.cmd");
    let windows = read(&root, "qa/release/build-release.ps1");
    let linux = read(&root, "build-release.sh");

    assert!(
        windows_launcher.contains("qa\\release\\build-release.ps1")
            && windows_launcher.contains("powershell.exe")
            && windows_launcher.contains("%*"),
        "the root Windows launcher must forward to the canonical PowerShell implementation"
    );

    for (name, source) in [("Windows", &windows), ("Linux", &linux)] {
        for required in [
            "run-full-validation",
            "--no-pause",
            "git status",
            "SOURCE_DATE_EPOCH",
            "worktree",
            "--release",
            "--locked",
            "--offline",
            "dashboard",
            "p2p_node",
            "CARGO_INCREMENTAL",
            "--remap-path-prefix",
            "SHA256",
            "reproducible-release",
            "BUILD-MANIFEST.txt",
            "SHA256SUMS.txt",
        ] {
            assert!(
                source.contains(required),
                "{name} canonical release runner is missing reproducibility guard: {required}"
            );
        }
        assert!(
            !source.contains("skip-validation") && !source.contains("skip_validation"),
            "{name} canonical release runner must not provide a path around full validation"
        );
        let worktree_adds = source.matches("worktree\", \"add\"").count()
            + source.matches("git worktree add").count();
        assert_eq!(
            worktree_adds, 2,
            "{name} runner must build from two independent clean worktrees"
        );
    }

    assert!(
        windows.contains("link-arg=/Brepro")
            && windows.contains("Get-FileHash")
            && windows.contains("Initialize-MsvcEnvironment"),
        "Windows releases must enable deterministic MSVC linking and compare SHA-256 hashes"
    );
    assert!(
        linux.contains("link-arg=-Wl,--build-id=sha1")
            && linux.contains("sha256sum")
            && linux.contains("cmp -s"),
        "Linux releases must use a deterministic build-id and compare the output byte-for-byte"
    );

    let gitignore = read(&root, ".gitignore");
    assert!(
        gitignore.lines().any(|line| line == "/dist/"),
        "canonical release outputs must stay out of the source working tree"
    );
}
