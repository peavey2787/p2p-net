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
    let windows_validation = read(&root, "run-full-validation.cmd");
    let linux_validation = read(&root, "run-full-validation.sh");

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
            "validation-source",
            "untracked-files=no",
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
            worktree_adds, 3,
            "{name} runner must validate one frozen snapshot and build from two independent clean worktrees"
        );
        assert!(
            source.contains("Full production validation from frozen snapshot"),
            "{name} runner must validate the frozen source snapshot instead of the mutable checkout"
        );
        assert!(
            source.contains("release-worktrees"),
            "{name} runner must place detached source worktrees outside the repository tree"
        );
        assert!(
            !source.contains("$ReproRoot \"validation-source\"")
                && !source.contains("$REPRO_ROOT/validation-source"),
            "{name} runner must not nest detached source worktrees below target/reproducible-release"
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

    assert!(
        windows_validation.contains("Cargo.lock.format-backup")
            && windows_validation.contains("Post-format lockfile verification")
            && windows_validation.contains("Cargo.lock.audit-backup")
            && windows_validation.contains("Post-audit lockfile verification")
            && windows_validation.contains("Cargo.lock.deny-backup")
            && windows_validation.contains("Post-deny lockfile verification")
            && windows_validation.contains("Cargo.lock.validation-baseline")
            && windows_validation.contains("fc /B Cargo.lock")
            && linux_validation.contains("Cargo.lock.format-backup")
            && linux_validation.contains("run_lockfile_readonly_tool cargo-audit")
            && linux_validation.contains("run_lockfile_readonly_tool cargo-deny")
            && linux_validation.contains("Cargo.lock.validation-baseline")
            && linux_validation.contains("cargo metadata --locked --format-version 1 >/dev/null")
            && linux_validation.contains("cmp -s \"$ROOT/Cargo.lock\""),
        "validation must transactionally restore lockfile writes from unlocked tooling, re-prove locked metadata after each tool, and retain a final Cargo.lock immutability backstop"
    );

    let gitignore = read(&root, ".gitignore");
    assert!(
        gitignore.lines().any(|line| line == "/dist/"),
        "canonical release outputs must stay out of the source working tree"
    );
}

#[test]
fn validation_evidence_wrapper_handles_empty_and_singleton_argument_sets() {
    let root = root();
    let wrapper = read(&root, "qa/evidence/run-validation-with-evidence.ps1");
    let linux_wrapper = read(&root, "qa/evidence/run-validation-with-evidence.sh");
    let release = read(&root, "qa/release/build-release.ps1");

    assert!(
        wrapper.contains("$Tokens = @()")
            && wrapper.contains("$Tokens = @($RawArgs -split")
            && wrapper.contains("$ErrorActionPreference = \"Stop\"")
            && !wrapper.contains("$Tokens = if ($RawArgs)"),
        "PowerShell must materialize validation argument tokens as an array even for zero/one token"
    );
    assert!(
        wrapper.contains("$markerName = if ($ExitCode -eq 0)")
            && wrapper.contains("$marker = Join-Path $RunDir $markerName")
            && wrapper.contains("$gitStatusState = if ($gitStatus.Count -eq 0)")
            && !wrapper.contains("$(if")
            && !wrapper.contains("Join-Path $RunDir (if"),
        "Windows PowerShell 5.1 must not receive a parenthesized if-statement as a Join-Path argument"
    );
    assert!(
        wrapper.contains("$PreValidationFingerprint")
            && wrapper.contains("post_validation_release_input_sha256=")
            && wrapper.contains("release_inputs_stable="),
        "Windows evidence must bind to the pre-validation source and record post-validation input drift"
    );
    assert!(
        linux_wrapper.contains("PRE_RELEASE_INPUT_SHA256")
            && linux_wrapper.contains("post_validation_release_input_sha256=")
            && linux_wrapper.contains("release_inputs_stable="),
        "Linux evidence must bind to the pre-validation source and record post-validation input drift"
    );
    assert!(
        release.contains("qa\\evidence\\recovered"),
        "release evidence lookup must include recovered machine transcripts"
    );
    let recovered = release
        .find("qa\\evidence\\recovered")
        .expect("recovered evidence root");
    let attested = release
        .find("qa\\evidence\\attestations")
        .expect("attestation evidence root");
    assert!(
        recovered < attested,
        "recovered machine transcripts must be preferred over user attestations"
    );
}

#[test]
fn android_release_requires_two_byte_identical_clean_builds() {
    let root = root();
    let windows = read(&root, "apps/android/build-android.ps1");
    let linux = read(&root, "apps/android/build-android.sh");
    let root_windows_launcher = read(&root, "build-android.cmd");
    assert!(root_windows_launcher.contains("build-android.ps1"));
    assert!(root_windows_launcher.contains("-Variant Release"));
    assert!(root_windows_launcher.contains("pause >nul"));
    assert!(root_windows_launcher.contains("exit /B %STATUS%"));

    for (name, source) in [("Windows", &windows), ("Linux", &linux)] {
        for required in [
            "build-a",
            "build-b",
            "--no-build-cache",
            "--no-configuration-cache",
            "--rerun-tasks",
            "CARGO_TARGET_DIR",
            "CARGO_INCREMENTAL",
            "SOURCE_DATE_EPOCH",
            "app-release-unsigned.apk",
            "app-release.aab",
            "SHA256SUMS.txt",
            "BUILD-MANIFEST.txt",
            "reproducible=true",
            "dist",
            "android",
        ] {
            assert!(
                source.contains(required),
                "{name} Android release builder missing reproducibility guard `{required}`"
            );
        }
    }

    assert!(windows.contains("Get-FileHash"));
    assert!(windows.contains("Test-FilesByteIdentical"));
    assert!(windows.contains("$buildA.ApkHash -ne $buildB.ApkHash"));
    assert!(windows.contains("$buildA.AabHash -ne $buildB.AabHash"));
    assert!(windows.contains("apk_byte_identical="));
    assert!(windows.contains("aab_byte_identical="));
    assert!(linux.contains("sha256sum"));
    assert!(linux.contains("cmp -s \"$APK_A\" \"$APK_B\""));
    assert!(linux.contains("cmp -s \"$AAB_A\" \"$AAB_B\""));
    assert!(linux.contains("$APK_HASH_A\" != \"$APK_HASH_B"));
    assert!(linux.contains("$AAB_HASH_A\" != \"$AAB_HASH_B"));
    assert!(linux.contains("apk_byte_identical=$APK_BYTE_IDENTICAL"));
    assert!(linux.contains("aab_byte_identical=$AAB_BYTE_IDENTICAL"));

    let validate_windows = read(&root, "apps/android/validate-android.ps1");
    let validate_linux = read(&root, "apps/android/validate-android.sh");
    for source in [validate_windows, validate_linux] {
        assert!(source.contains("reproducible=true"));
        assert!(source.contains("apk_byte_identical=true"));
        assert!(source.contains("aab_byte_identical=true"));
    }
}

#[test]
fn crates_io_release_has_no_root_patch_dependency() {
    let root = root();
    let manifest = read(&root, "Cargo.toml");
    let gitignore = read(&root, ".gitignore");
    let support = read(&root, "external/libp2p-webrtc/Cargo.toml");
    let windows = read(&root, "qa/release/package-crates.ps1");
    let linux = read(&root, "qa/release/package-crates.sh");
    let windows_launcher = read(&root, "package-crates.cmd");
    let fingerprint_windows = read(&root, "qa/evidence/source-fingerprint.ps1");
    let fingerprint_linux = read(&root, "qa/evidence/source-fingerprint.sh");
    assert!(windows_launcher.contains("package-crates.ps1"));
    assert!(windows_launcher.contains("pause >nul"));
    assert!(windows_launcher.contains("exit /b %RC%"));
    assert!(
        fingerprint_windows.contains("\".cargo/config.toml\"")
            && fingerprint_linux.contains(".cargo/config.toml"),
        "release-input fingerprint must include build-affecting .cargo/config.toml"
    );

    assert!(manifest.contains("publish = true"));
    assert!(manifest.contains("repository = \"https://github.com/peavey2787/p2p-net\""));
    assert!(manifest.contains("documentation = \"https://docs.rs/p2p-net\""));
    assert!(!manifest.contains("[patch.crates-io]"));
    assert!(
        gitignore.lines().any(|line| line == "!/.cargo/")
            && gitignore
                .lines()
                .any(|line| line == "!/.cargo/config.toml"),
        "release-critical .cargo/config.toml must not be hidden by the generic dot-directory ignore rule"
    );
    assert!(manifest.contains("package = \"p2p-net-webrtc\""));
    assert!(manifest.contains("version = \"0.1.0\""));
    assert!(manifest.contains("members = [\"apps/android/native\"]"));
    assert!(manifest.contains("exclude = [\"qa/fuzz\", \"external/libp2p-webrtc\"]"));
    for excluded in [
        "\".cargo/**\"",
        "\"external/libp2p-dns/**\"",
        "\"external/libp2p-mdns-placeholder/**\"",
        "\"external/libp2p-webrtc/**\"",
    ] {
        assert!(
            manifest.contains(excluded),
            "workspace-only source must be excluded from crates.io payload: {excluded}"
        );
    }
    assert!(support.contains("name = \"p2p-net-webrtc\""));
    assert!(support.contains("publish = true"));
    assert!(support.contains("documentation = \"https://docs.rs/p2p-net-webrtc\""));
    assert!(
        gitignore.lines().any(|line| line == "/external/**/target/")
            && gitignore
                .lines()
                .any(|line| line == "/external/**/Cargo.lock"),
        "generated Cargo state under external/ must stay outside release fingerprints"
    );

    for (name, source) in [("Windows", &windows), ("Linux", &linux)] {
        for required in [
            "cargo",
            "metadata --locked",
            "package",
            "p2p-net-webrtc-0.1.0.crate",
            "p2p-net-0.1.0.crate",
            "[patch.crates-io]",
            "path",
            "SHA256SUMS.txt",
            "PUBLISH-ORDER.txt",
            "p2p-net-package-consumer-smoke",
            "generate-lockfile",
            "compile packaged p2p-net as downstream consumer",
            "--registry",
            "crates-io",
            "p2p-net-webrtc",
            "p2p-net",
            "external/libp2p-dns",
            "external/libp2p-mdns-placeholder",
        ] {
            assert!(
                source.contains(required),
                "{name} crates.io package runner missing `{required}`"
            );
        }
    }
    for (name, source) in [("Windows", &windows), ("Linux", &linux)] {
        assert!(
            source.contains("package-crates") && source.contains("CARGO_TARGET_DIR"),
            "{name} package validation must isolate Cargo package output under the ignored root target tree"
        );
        assert!(
            !source.contains("external/libp2p-webrtc/target/package")
                && !source.contains("external\\libp2p-webrtc\\target\\package"),
            "{name} package validation must never write package output under release-fingerprinted external/ source"
        );
        assert!(
            !source.contains("--no-verify"),
            "{name} package validation must fully verify the normalized crates"
        );
        assert!(
            source.contains("patch.crates-io.p2p-net-webrtc.path"),
            "{name} package validation must resolve the unpublished companion only through a command-line Cargo patch"
        );
        assert!(
            source.matches("p2p-net-webrtc").count() >= 2 && source.matches("p2p-net").count() >= 2,
            "{name} package validation must qualify both interdependent crates"
        );
    }
    assert!(windows.contains("Invoke-NativeCapture"));
    for required in [
        "Invoke-CheckedLive",
        "Write-Progress",
        "WaitForExit(1000)",
        "RedirectStandardOutput = $false",
        "RedirectStandardError = $false",
        "[1/$PackageProgressPhases]",
        "[5/$PackageProgressPhases]",
    ] {
        assert!(
            windows.contains(required),
            "Windows crates.io package runner must expose live/progress output via `{required}`"
        );
    }
    assert!(windows.contains("Get-FileHash"));
    assert!(linux.contains("sha256sum"));
}
