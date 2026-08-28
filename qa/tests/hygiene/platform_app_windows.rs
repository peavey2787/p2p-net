use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn android_windows_bootstrap_and_run_scripts_are_pinned_and_idempotent() {
    let root = root();
    let setup = fs::read_to_string(root.join("apps/android/setup-android.ps1"))
        .expect("Android Windows setup script");
    for required in [
        "1.98.0",
        "4.1.2",
        "9.5.0",
        "android-37.0",
        "platforms;android-37.0",
        "platforms\\android-37.0\\android.jar",
        "retrying platforms;android-37.0 with sdkmanager channel 3",
        "build-tools;37.0.0",
        "system-images;android-37.0;google_apis_ps16k;x86_64",
        "--channel=3",
        "28.2.13676358",
        "3.22.1",
        "commandlinetools-win-15859902_latest.zip",
        "90ae805d20434428bffcb699c290860f19bb5f66a67e6b330067e3de801fb04a",
        "553c78f50dafcd54d65b9a444649057857469edf836431389695608536d6b746",
        "Microsoft.OpenJDK.17",
        "Microsoft.VisualStudio.2022.BuildTools",
        "HypervisorPlatform",
        "P2PNet_API_37",
        "cargo metadata --locked",
    ] {
        assert!(
            setup.contains(required),
            "Android setup script missing pinned/bootstrap input: {required}"
        );
    }

    assert!(
        setup.contains("Invoke-NativeCapture"),
        "Android setup must capture native stderr without PowerShell NativeCommandError termination"
    );
    assert!(
        !setup.contains("-version 2>&1"),
        "Android setup must not merge Java/native stderr through PowerShell under Stop semantics"
    );
    assert!(
        !setup.contains("metadata --locked --format-version 1 *> $null"),
        "Android setup must not redirect Cargo stderr through PowerShell under Stop semantics"
    );
    assert!(
        setup.contains(
            "Invoke-NativeCapture \"cargo.exe\" \"+$RustVersion metadata --locked --format-version 1\""
        ),
        "Android setup must capture the locked Cargo metadata preflight with the native-process helper"
    );

    assert!(
        !setup.contains("Test-Path $sdkManager -PathType Leaf -and"),
        "Android setup must parenthesize Test-Path before boolean -and operators"
    );
    assert!(
        !setup.contains("sdk install") && !setup.contains("--canary"),
        "Windows setup must not invoke the crashing preview Android CLI package installer",
    );
    for required in [
        "Repair-KnownBrokenSdkRootState",
        "Get-KnownBrokenSdkRoot",
        "invalid SDK path created by an older p2p-net launcher",
        "Removing stale CMake state that referenced the invalid SDK path",
    ] {
        assert!(
            setup.contains(required),
            "Android setup must repair the historical -AvdName SDK-root poisoning: {required}",
        );
    }

    let build_ps1 = fs::read_to_string(root.join("apps/android/build-android.ps1"))
        .expect("Android Windows build script");
    assert!(build_ps1.contains("Remove-PoisonedCmakeState"));
    assert!(build_ps1.contains("Get-KnownBrokenSdkRoot"));
    assert!(
        !build_ps1.contains("metadata --locked --format-version 1 *> $null"),
        "Android build must not redirect Cargo stderr through PowerShell under Stop semantics"
    );
    assert!(
        build_ps1.contains(
            "Invoke-NativeCapture \"cargo.exe\" \"+1.98.0 metadata --locked --format-version 1\""
        ),
        "Android build must capture the locked Cargo metadata preflight with the native-process helper"
    );

    let setup_cmd = fs::read_to_string(root.join("apps/android/setup-android.cmd"))
        .expect("Android Windows setup CMD wrapper");
    assert!(
        setup_cmd.contains("pause"),
        "Android setup CMD must pause before closing"
    );

    let run = fs::read_to_string(root.join("apps/android/run-android.ps1"))
        .expect("Android Windows run script");
    for required in [
        "build-android.ps1",
        "-Variant Debug",
        "adb.exe",
        "\"install\", \"-r\", \"-t\"",
        "\"am\", \"start\", \"-W\"",
        "P2PNet_API_37",
        "io.github.peavey2787.p2pnet",
    ] {
        assert!(
            run.contains(required),
            "Android run script missing build/deploy contract: {required}"
        );
    }
    assert!(
        run.contains("Invoke-NativeCapture"),
        "Android run script must capture emulator stderr without PowerShell NativeCommandError termination"
    );
    assert!(
        run.contains("Invoke-AdbCapture") && run.contains("Require-CapturedSuccess"),
        "Android run script must capture adb stdout/stderr without PowerShell NativeCommandError termination",
    );
    assert!(
        !run.contains("& $adb") && !run.contains("$adb start-server *> $null"),
        "Android run script must not invoke adb directly under ErrorActionPreference=Stop",
    );
    assert!(
        run.contains("$setupParameters = @{ AvdName = $AvdName }")
            && run.contains("@setupParameters"),
        "Android run script must use named hashtable splatting when invoking setup",
    );
    assert!(
        !run.contains("$setupArgs = @(\"-AvdName\", $AvdName)"),
        "Array splatting must not turn -AvdName into setup's positional SdkRoot argument",
    );
    assert!(run.contains("Test-KnownBrokenSdkRoot"));
    assert!(run.contains("Android\\Sdk"));
    let run_cmd = fs::read_to_string(root.join("apps/android/run-android.cmd"))
        .expect("Android Windows run CMD wrapper");
    assert!(
        run_cmd.contains("pause"),
        "Android run CMD must pause before closing"
    );
}
