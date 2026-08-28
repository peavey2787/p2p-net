use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn desktop_and_android_apps_have_explicit_platform_owners() {
    let root = root();
    for required in [
        "apps/README.md",
        "apps/windows/p2p_node.rs",
        "apps/windows/p2p_node/view/mod.rs",
        "apps/android/native/Cargo.toml",
        "apps/android/native/src/lib.rs",
        "apps/android/app/src/main/AndroidManifest.xml",
        "apps/android/app/src/main/cpp/p2p_jni.cpp",
        "apps/android/app/src/main/java/io/github/peavey2787/p2pnet/NodeRepository.kt",
        "apps/android/app/src/main/java/io/github/peavey2787/p2pnet/P2PNodeService.kt",
        "apps/android/IMPLEMENTATION_PLAN.md",
        "apps/android/setup-android.ps1",
        "apps/android/setup-android.cmd",
        "apps/android/run-android.ps1",
        "apps/android/run-android.cmd",
        "apps/android/qa/resource-soak.ps1",
        "apps/android/qa/resource-soak.sh",
        "apps/android/qa/lifecycle-cycles.ps1",
        "apps/android/qa/lifecycle-cycles.sh",
        "apps/android/qa/verify-rust-tooling.ps1",
        "apps/android/qa/verify-rust-tooling.sh",
        "crates/platform/android.rs",
        "crates/platform/desktop.rs",
        "crates/platform/ios.rs",
    ] {
        assert!(
            root.join(required).is_file(),
            "missing platform owner: {required}"
        );
    }
    assert!(
        !root.join("examples/p2p_node.rs").exists(),
        "desktop application must not drift back into examples/"
    );
    assert!(
        !root.join("crates/platform/mobile.rs").exists(),
        "Android and iOS platform policy must remain separate"
    );
}

#[test]
fn shared_crates_do_not_absorb_android_ui_or_jni_code() {
    let root = root();
    let mut rust_files = Vec::new();
    collect_rust_files(&root.join("crates"), &mut rust_files);
    for path in rust_files {
        let source = fs::read_to_string(&path).expect("read shared Rust source");
        for forbidden in [
            "JNIEnv",
            "jni::",
            "android.app.",
            "androidx.",
            "Jetpack Compose",
            "P2PNodeService",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} leaked Android app concern `{forbidden}` into shared crates",
                path.display()
            );
        }
    }

    let core = fs::read_to_string(root.join("crates/lib.rs")).expect("shared core lib");
    assert!(core.contains("#![forbid(unsafe_code)]"));
    let android_bridge = fs::read_to_string(root.join("apps/android/native/src/lib.rs"))
        .expect("Android Rust bridge");
    assert!(android_bridge.contains("#![deny(unsafe_op_in_unsafe_fn)]"));
}

#[test]
fn android_host_keeps_desktop_low_overhead_sampling_and_bounded_state() {
    let root = root();
    let bridge = fs::read_to_string(root.join("apps/android/native/src/lib.rs"))
        .expect("Android Rust bridge");
    for required in [
        "RUNTIME_WORKER_THREADS: usize = 2",
        "RUNTIME_MAX_BLOCKING_THREADS: usize = 4",
        "RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2)",
        "MESSAGE_QUEUE_CAPACITY: usize = 128",
        "MAX_SUBSCRIPTIONS: usize = 64",
        "MAX_DRAIN_MESSAGES: usize = 64",
        "MAX_CONFIG_JSON_BYTES: usize = 256 * 1024",
        "MAX_DATA_DIR_BYTES: usize = 4 * 1024",
        "MAX_MULTIADDR_BYTES: usize = 4 * 1024",
        "MAX_PEER_ID_BYTES: usize = 256",
        "MAX_RESPONSE_JSON_BYTES: usize = 4 * 1024 * 1024",
        "MAX_BRIDGE_PEERS: usize = 512",
        "MAX_APP_MESSAGE_BYTES",
        "p2p_android_validate_config",
        "p2p_android_bridge_stats_json",
        "catch_unwind",
    ] {
        assert!(
            bridge.contains(required),
            "missing Android native bound: {required}"
        );
    }

    let repository = fs::read_to_string(
        root.join("apps/android/app/src/main/java/io/github/peavey2787/p2pnet/NodeRepository.kt"),
    )
    .expect("Android repository");
    for required in [
        "POLL_INTERVAL_MS = 1_000L",
        "PEER_REFRESH_INTERVAL_MS = 5_000L",
        "appendBoundedMessages",
        "nativeOperationMutex",
        "refreshMutex.tryLock()",
        "refreshRequested.compareAndSet(false, true)",
        "cancelAndJoin",
        "NativeNode.revision()",
        "revision != _state.value.revision",
        "NativeNode.pendingMessageCount()",
        "BRIDGE_REFRESH_INTERVAL_MS = 10_000L",
        "MAX_TRIMMED_MESSAGES = 20",
        "MAX_TRIMMED_MESSAGE_PAYLOAD_BYTES = 1024 * 1024",
        "trimTransientUiState",
    ] {
        assert!(
            repository.contains(required),
            "missing Android sampling guard: {required}"
        );
    }

    let service = fs::read_to_string(
        root.join("apps/android/app/src/main/java/io/github/peavey2787/p2pnet/P2PNodeService.kt"),
    )
    .expect("Android foreground service");
    assert!(service.contains("START_STICKY"));
    assert!(service.contains("stopSelfResult(startId)"));
    assert!(service.contains("unregisterNetworkCallback"));
    assert!(!service.contains("WakeLock"));
    assert!(service.contains("FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE"));
    for required in [
        "WifiManager.MulticastLock",
        "createMulticastLock",
        "MULTICAST_LOCK_TAG",
        "acquireMulticastLock()",
        "releaseMulticastLock()",
    ] {
        assert!(
            service.contains(required),
            "Android LAN discovery lifecycle guard missing: {required}"
        );
    }

    let jni = fs::read_to_string(root.join("apps/android/app/src/main/cpp/p2p_jni.cpp"))
        .expect("Android JNI shim");
    for required in [
        "kMaxPayloadBytes = 1024 * 1024",
        "kMaxConfigBytes = 256 * 1024",
        "p2p_android_validate_config",
        "config_utf8.size()",
        "bytes.size()",
        "codepoint >= 0xDC00",
    ] {
        assert!(
            jni.contains(required),
            "missing Android JNI guard: {required}"
        );
    }
    assert!(
        !jni.contains("GetStringUTFChars"),
        "Android JNI must not rely on modified-UTF-8 C-string scanning"
    );
}

#[test]
fn android_build_inputs_are_pinned_and_lan_permission_is_explicit() {
    let root = root();
    let build = fs::read_to_string(root.join("apps/android/app/build.gradle.kts"))
        .expect("Android Gradle app config");
    for required in [
        "version = release(37)",
        "minorApiLevel = 0",
        "targetSdk = 37",
        "ndkVersion = \"28.2.13676358\"",
        "arm64-v8a",
        "x86_64",
        "verifyRustAndroidTooling",
        "rustupExecutable",
        "\"1.98.0\"",
        "\"cargo\"",
        "\"ndk\"",
        "--release",
        "--locked",
        "CARGO_INCREMENTAL",
        "kotlin-test-junit:2.3.21",
    ] {
        assert!(
            build.contains(required),
            "missing Android build pin: {required}"
        );
    }

    assert!(
        !build.contains("doLast") && !build.contains("providers.exec"),
        "Android Gradle tooling verification must remain configuration-cache serializable",
    );

    let dashboard =
        fs::read_to_string(root.join(
            "apps/android/app/src/main/java/io/github/peavey2787/p2pnet/ui/DashboardScreen.kt",
        ))
        .expect("Android dashboard UI");
    assert!(
        !dashboard.contains("import androidx.compose.foundation.layout.weight"),
        "Compose weight must resolve from RowScope/ColumnScope rather than the internal top-level symbol",
    );
    assert!(dashboard.contains("Modifier.weight(1f)"));

    let cmake = fs::read_to_string(root.join("apps/android/app/src/main/cpp/CMakeLists.txt"))
        .expect("Android CMake config");
    for required in [
        "IMPORTED_NO_SONAME TRUE",
        "SKIP_BUILD_RPATH TRUE",
        "libp2p_net_android.so",
    ] {
        assert!(
            cmake.contains(required),
            "Android native link config missing runtime-safe dependency guard: {required}"
        );
    }

    for verifier in [
        "apps/android/qa/verify-rust-tooling.ps1",
        "apps/android/qa/verify-rust-tooling.sh",
    ] {
        let source =
            fs::read_to_string(root.join(verifier)).expect("Android Rust tooling verifier");
        for required in ["1.98.0", "4.1.2", "rustup"] {
            assert!(
                source.contains(required),
                "{verifier} missing pinned tooling contract: {required}"
            );
        }
    }

    let root_build = fs::read_to_string(root.join("apps/android/build.gradle.kts"))
        .expect("Android root Gradle config");
    for required in ["9.3.0", "2.3.21", "kotlin-gradle-plugin:2.3.21"] {
        assert!(
            root_build.contains(required),
            "missing Android plugin pin: {required}"
        );
    }
    assert!(
        !root_build.contains("org.jetbrains.kotlin.android"),
        "AGP 9.3 must use built-in Kotlin rather than the incompatible kotlin-android plugin"
    );

    let manifest = fs::read_to_string(root.join("apps/android/app/src/main/AndroidManifest.xml"))
        .expect("Android manifest");
    assert!(manifest.contains("android.permission.ACCESS_LOCAL_NETWORK"));
    assert!(manifest.contains("android.permission.CHANGE_WIFI_MULTICAST_STATE"));

    let activity = fs::read_to_string(
        root.join("apps/android/app/src/main/java/io/github/peavey2787/p2pnet/MainActivity.kt"),
    )
    .expect("Android main activity");
    for required in [
        "Manifest.permission.ACCESS_LOCAL_NETWORK",
        "Build.VERSION.SDK_INT >= 37",
        "permissionLauncher.launch",
        "updateLocalNetworkPermission",
    ] {
        assert!(
            activity.contains(required),
            "Android local-network runtime permission guard missing: {required}"
        );
    }

    assert!(manifest.contains("FOREGROUND_SERVICE_CONNECTED_DEVICE"));
    assert!(manifest.contains("android:exported=\"false\""));
    assert!(manifest.contains("android:allowBackup=\"false\""));
}

#[test]
fn android_release_scripts_fail_closed_on_pinned_tooling_and_device_qa_is_manual() {
    let root = root();
    for script in [
        "apps/android/build-android.sh",
        "apps/android/build-android.ps1",
    ] {
        let source = fs::read_to_string(root.join(script)).expect("Android build script");
        for required in [
            "1.98.0",
            "4.1.2",
            "9.5.0",
            "android-37.0",
            "28.2.13676358",
            "3.22.1",
            "cargo metadata --locked",
            "ANDROID_NDK_HOME",
        ] {
            assert!(
                source.contains(required),
                "{script} missing pinned input: {required}"
            );
        }
    }

    let windows_build = fs::read_to_string(root.join("apps/android/build-android.ps1"))
        .expect("Android Windows build script");
    for required in [
        "Normalize-FutureBuildInputTimestamps",
        "future-dated Android build input(s)",
        "LastWriteTime",
        "AddSeconds(5)",
        "app\\.cxx",
        "future-dated CMake input recovery",
    ] {
        assert!(
            windows_build.contains(required),
            "Windows Android build missing archive timestamp recovery: {required}"
        );
    }

    let resource_qa = fs::read_to_string(root.join("apps/android/qa/resource-soak.ps1"))
        .expect("Android resource soak");
    assert!(resource_qa.contains("dumpsys meminfo"));
    assert!(resource_qa.contains("dumpsys cpuinfo"));
    assert!(resource_qa.contains("Threads:"));
    assert!(resource_qa.contains("System.Diagnostics.ProcessStartInfo"));
    assert!(!resource_qa.contains("& adb"));

    let lifecycle_windows = fs::read_to_string(root.join("apps/android/qa/lifecycle-cycles.ps1"))
        .expect("Android Windows lifecycle cycles");
    assert!(lifecycle_windows.contains("System.Diagnostics.ProcessStartInfo"));
    assert!(!lifecycle_windows.contains("& adb"));

    let lifecycle_qa = fs::read_to_string(root.join("apps/android/qa/lifecycle-cycles.sh"))
        .expect("Android lifecycle cycles");
    for required in ["TOTAL PSS:", "TOTAL RSS:", "/proc/$PID/fd", "Threads:"] {
        assert!(
            lifecycle_qa.contains(required),
            "lifecycle QA missing: {required}"
        );
    }

    let models = fs::read_to_string(
        root.join("apps/android/app/src/main/java/io/github/peavey2787/p2pnet/Models.kt"),
    )
    .expect("Android models");
    for required in [
        "Base64.getDecoder().decode",
        "payload_base64",
        "MAX_NATIVE_RESPONSE_BYTES = 4 * 1024 * 1024",
        "MAX_UI_MESSAGE_PAYLOAD_BYTES = 4 * 1024 * 1024",
    ] {
        assert!(
            models.contains(required),
            "Android model bound missing: {required}"
        );
    }

    let validation = fs::read_to_string(root.join("apps/android/validate-android.sh"))
        .expect("Android validation script");
    assert!(validation.contains("run-full-validation.sh"));
    assert!(validation.contains(":app:lintRelease"));
    assert!(validation.contains(":app:testReleaseUnitTest"));
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source directory") {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}
