# Android native app

`apps/android` is the native Android host for the same hardened Rust `p2p-net` core used by the desktop `p2p_node` application. It is not a second networking implementation.

## Architecture

- `native/` is a small Rust `cdylib` bridge. Unsafe pointer handling is confined to this crate; the shared core keeps `#![forbid(unsafe_code)]`.
- `app/src/main/cpp/` is the JNI conversion shim. Kotlin/Android types never enter `crates/`.
- `app/src/main/java/.../NativeNode.kt` is the JNI declaration only.
- `NodeRepository` owns sampling, bounded UI state, and API calls.
- `P2PNodeService` owns Android foreground-service/network lifecycle.
- `ui/` renders the dashboard, peers, application messages, and config editor with Jetpack Compose.
- `crates/platform/android.rs` contains only Android runtime facts and app-private durable storage for the shared core; storage keys cannot escape the app root and symlinked ancestors are rejected.

The Android app defaults to `NodeProfile::Full` while the foreground service is running. Actual public reachability is still discovered by the core's normal AutoNAT/relay logic; the Android adapter does not assume that a phone is publicly reachable.

## Pinned build inputs

- Rust 1.98.0 from the repository `rust-toolchain.toml`
- Android compile/target SDK 37
- Android NDK 28.2.13676358
- Android Gradle Plugin 9.3.0
- Gradle 9.5.0
- JDK 17
- Kotlin 2.3.21
- cargo-ndk 4.1.2
- ABIs: `arm64-v8a` and `x86_64`

AGP 9.3 uses its built-in Kotlin integration; the obsolete `org.jetbrains.kotlin.android` plugin is deliberately not applied. KGP/Compose compiler 2.3.21 remains explicitly pinned.

The source tree does not vendor a generated Gradle wrapper JAR. `build-android.ps1`/`.sh` fail closed unless Gradle 9.5.0, JDK 17, SDK 37, NDK 28.2.13676358, CMake 3.22.1, the two Rust Android targets, and cargo-ndk 4.1.2 are present, avoiding an unverified binary bootstrap in the repository.

## Windows setup, build, and run

On Windows 11, the repository includes an idempotent setup script. It checks existing tools first and only installs missing pinned prerequisites. It covers Rust 1.98.0 plus both Android targets, cargo-ndk 4.1.2, Visual Studio C++ Build Tools, JDK 17, Gradle 9.5.0, Android command-line tools, SDK/API 37, Build Tools 37.0.0, NDK 28.2.13676358, CMake 3.22.1, platform-tools/ADB, the Android Emulator, the current API 37.0 Google APIs 16 KB-page-size x86_64 system image, and the `P2PNet_API_37` AVD.

From the repository root:

```cmd
apps\android\setup-android.cmd
```

The setup script uses the configured `ANDROID_SDK_ROOT`/`ANDROID_HOME` when present, otherwise an existing `apps\android\local.properties`, otherwise `%LOCALAPPDATA%\Android\Sdk`. Downloaded standalone Gradle and Android command-line tools are SHA-256 verified before extraction. Missing JDK 17 and Visual Studio Build Tools are installed through Windows Package Manager (`winget`).

Android SDK Platform 37 is published under the minor-versioned package `platforms;android-37.0` (installed as `platforms/android-37.0`), so the app explicitly selects API 37 minor level 0 in the Gradle compile SDK DSL. On Windows, setup uses the pinned command-line-tools `sdkmanager --channel=3` path directly for API 37.0 packages. This avoids the current preview `android-cli.exe` crash observed after successful downloads while still installing the exact platform and `system-images;android-37.0;google_apis_ps16k;x86_64` emulator image. Setup verifies that `platforms/android-37.0/android.jar` actually exists after installation and retries that exact package if necessary. An already-installed legacy API 37 Google APIs x86_64 image is accepted rather than downloaded again.

To build a debug APK, prefer a connected/authorized physical Android device, install the APK, and launch the app in one command:

```cmd
apps\android\run-android.cmd
```

If no physical device is ready, the run script starts the `P2PNet_API_37` emulator automatically. Use PowerShell-style parameters through the CMD wrapper as normal CMD arguments, for example `apps\android\run-android.cmd -Device <adb-serial>` or `apps\android\run-android.cmd -ColdBoot`. The setup checks emulator acceleration and, when Windows Hypervisor Platform is disabled, enables it through a single UAC-elevated DISM call. If Windows requires a reboot, setup stops with a clear restart-and-rerun message. CPU virtualization still has to be enabled in UEFI/BIOS. A physical device requires Developer options and USB debugging plus any OEM USB driver Windows needs.

The Windows `.cmd` setup and run launchers always pause before closing, including after failures, so diagnostics remain visible when they are started by double-clicking. Native version/acceleration probes capture stderr directly rather than routing it through Windows PowerShell error semantics; this is required because tools such as `java -version` legitimately write their version text to stderr.

Use `apps\android\setup-android.cmd -NoEmulator` if you only intend to deploy to a physical device. Use `apps\android\run-android.cmd -NoSetup` after the environment is already prepared and you do not want the idempotent prerequisite check on that run.

## Release build

The lower-level pinned release build remains available from the repository root:

```powershell
.\apps\android\build-android.ps1
```

or:

```bash
./apps/android/build-android.sh
```

The Gradle build compiles the Rust `cdylib` for both supported ABIs before CMake links the JNI shim. The Rust `cdylib` is declared to CMake as an imported library without an ELF SONAME, so the JNI shim records `libp2p_net_android.so` by name rather than embedding the host `jniLibs` path in `DT_NEEDED`; build RPATHs are disabled for the shim for the same reason. Release mode performs **two clean builds** with separate Cargo target directories, Gradle build/configuration caches disabled, and all tasks rerun. The build fails unless both the unsigned APK and AAB are byte-for-byte identical by SHA-256. Verified artifacts are copied to `dist/android/` with `SHA256SUMS.txt` and `BUILD-MANIFEST.txt`. Signing keys are intentionally deployment-owned and are not accepted or stored by the repository.

For release qualification, run `validate-android.ps1`/`.sh`. That runs the full root production validation first, then the pinned two-build reproducible Android release, Android Lint, and release JVM tests. Build A and build B must match both by direct byte comparison and SHA-256 for the unsigned APK and AAB. A successful release leaves the verified unsigned APK/AAB and their reproducibility manifest under `dist/android/`.

## Runtime/performance design

The Android host preserves the desktop dashboard's low-overhead behavior instead of continuously serializing the large node snapshot:

- read the atomic snapshot revision once per second;
- clone/serialize a snapshot only after the revision changes;
- refresh peer details at most every five seconds unless explicitly requested;
- check the pending-message count before crossing JNI with message JSON;
- cap the native application-message queue at 128 / 8 MiB and the UI message history at 100 / 4 MiB;
- drain at most 64 messages and 2 MiB of payload at a time;
- cap subscriptions at 64, peer-detail transfers at 512 peers, and native JSON responses at 4 MiB;
- use a two-worker Tokio runtime with four blocking threads maximum and a bounded two-second runtime shutdown;
- never hold an Android wake lock;
- release the `ConnectivityManager.NetworkCallback` when the service dies;
- trim nonessential peer/message presentation caches when Android signals low memory;
- cap editable JNI/config/multiaddr/peer/topic/payload inputs before crossing the native boundary;
- keep Activity objects out of process-wide state by storing only `applicationContext`-derived paths/preferences;
- serialize native operations with lifecycle transitions so runtime teardown cannot race an in-flight JNI command;
- conflate overlapping refresh requests instead of queueing work during network callback storms;
- cancel and join the sampler before stopping/restarting the native runtime;
- stop the foreground service when native startup fails instead of leaving an idle service alive;
- run blocking JNI operations off the Compose/UI thread.

These are host-side protections in addition to the core's bounded peer/replay/reputation/transport state, coalesced persistence, and skipped missed ticks.

## Android 17 local-network permission

Target SDK 37 requires the `ACCESS_LOCAL_NETWORK` runtime permission for direct LAN access. The app requests it on Android 17 before starting the full node. Internet/relay connectivity remains separate from that permission.

Physical-device evidence helpers live under `qa/`. See `IMPLEMENTATION_PLAN.md` for the transport, lifecycle, hostile-input, and 24-hour resource qualification required before calling a particular Android APK production-qualified.
