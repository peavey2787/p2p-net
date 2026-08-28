# Android full-node implementation and qualification plan

## Goal

Ship a native Android full-node host that uses the exact same `p2p-net` protocol/runtime core as the desktop application, with Android-specific code confined to `apps/android` and `crates/platform/android.rs`. No networking protocol or security logic is duplicated in Kotlin/JNI.

## Implemented foundation

1. **Repository/platform separation**
   - Desktop application moved to `apps/windows/p2p_node.rs` and its focused `view/` modules.
   - Native Android application lives under `apps/android`.
   - Shared Rust core stays under `crates/`.
   - `crates/platform/{android,desktop,ios}.rs` keep platform policies separate.
   - Android JNI/Compose/service symbols are prohibited from `crates/` by QA.

2. **Native Rust Android bridge**
   - `cdylib` workspace member with a narrow C ABI around node start/stop, snapshots, peers, dial/disconnect, subscribe, broadcast, and addressed send.
   - Full node profile by default while foreground service is active.
   - C ABI catches panics before they can unwind through JNI.
   - UTF-8 and payload lengths are validated before Rust dereferences foreign pointers.
   - Native incoming-message queue is bounded by both count and bytes and drops the oldest retained message at capacity while reporting a cumulative drop counter.
   - Active subscription tasks are capped at 64, native JSON responses are capped at 4 MiB, and runtime shutdown has a two-second bound after orderly node shutdown.
   - Foreign strings/payloads use pointer+length ABIs with pre-dereference size checks; JNI does not use modified-UTF-8 C-string scanning.
   - Slow node API operations clone the `NodeHandle` before awaiting so the process-wide controller mutex is not held across dial/query work.

3. **Android lifecycle host**
   - Dedicated foreground service, independent of Activity lifetime.
   - Android 17 local-network permission request.
   - Connectivity callback is registered exactly once per service and unregistered on destruction.
   - No wake lock and no hot polling loop; overlapping refresh requests are conflated rather than queued.
   - JNI/node operations are serialized against start/stop/restart so the Tokio runtime cannot be torn down under an in-flight command.
   - Sampler cancellation is joined before runtime teardown, and failed native startup stops the foreground service instead of leaving an idle service alive.
   - App-private identity/peer-cache storage is create-only for secrets, rejects path traversal, final symlinks and symlinked parent directories, and syncs secret writes.

4. **Desktop-equivalent operator UI**
   - Compose dashboard for identity/network/reachability/transports/peer/relay/DCUtR/message counters and recent events.
   - Peer dial/disconnect view.
   - Application subscribe/broadcast/addressed-send view.
   - NodeConfig JSON editor validates the candidate config before stopping a running node and rolls back to the previous configuration if restart fails.
   - Dashboard exposes bridge queue/drop/thread limits so resource pressure is visible without a profiler.

5. **CPU/memory protections**
   - Revision-only 1 Hz probe, matching the desktop dashboard strategy.
   - Full snapshot transfer only on revision change.
   - Peer-detail refresh capped at 5 seconds unless manually requested.
   - Native queue 128 / 8 MiB; JNI drains 64 / 2 MiB max; UI history 100 / 4 MiB, with low-memory trimming to 20 / 1 MiB.
   - Native response JSON is capped at 4 MiB, peer-detail transfers are capped at 512 entries before JSON serialization, and payloads cross JNI as bounded base64 with declared-length validation.
   - Subscriptions are capped at 64; Tokio workers are fixed at 2, the blocking pool is capped at 4, and runtime shutdown has a two-second bound.
   - Incremental release compilation is disabled.
   - Activity references are never held globally; service scope/callbacks are cancelled deterministically.
   - `onLowMemory` trims transient UI caches while leaving authoritative bounded Rust state intact.
   - Physical-device resource/lifecycle evidence scripts live under `apps/android/qa/` and are intentionally kept out of normal CI.

## Qualification still required on real Android hardware

The implementation should not be labeled Android-production-qualified until every gate below is green on the same commit.

### A. Build and static gates

- Root `run-full-validation` green on Windows and Linux.
- `apps/android/validate-android.ps1` or `.sh` green with SDK 37/NDK 28.2/JDK 17/Gradle 9.5.0.
- Release APK/AAB produced for `arm64-v8a`; emulator build produced for `x86_64`.
- Android Lint has zero release-blocking findings.
- Native symbols inspected so only intended JNI/C ABI surface is exported.
- Dependency report reviewed for new vulnerable, unsound, or unmaintained Android dependencies.

### B. Functional transport matrix

Run peer-to-peer interoperability against a validated desktop node for each supported path:

- TCP direct dial/listen.
- QUIC direct dial/listen.
- WebSocket where enabled by configuration.
- WebRTC transport paths supported by the shared core.
- relay reservation and relayed application traffic.
- DCUtR upgrade with relay fallback retained after failed direct upgrade.
- public bootstrap, DHT provider discovery, rendezvous, and cached-peer resurrection.
- application subscribe, broadcast, addressed send, disconnect, restart, and identity persistence.

Test on Wi-Fi LAN, Wi-Fi behind NAT, cellular/CGNAT, and Wi-Fi-to-cellular/cellular-to-Wi-Fi transitions.

### C. Lifecycle and permission matrix

- Android 12, 13, 14, 15, 16, and 17 physical-device coverage where available; API 37 emulator is mandatory.
- Android 17 `ACCESS_LOCAL_NETWORK` grant, denial, later grant, and revocation.
- notification permission denial must not crash the node lifecycle.
- rotate Activity, background/foreground repeatedly, swipe UI away while service remains active, stop from UI, and relaunch.
- OS process death/restart must not corrupt identity or peer cache.
- Doze/battery-saver observation without wake locks.
- network callback registration/unregistration verified across 100 service start/stop cycles; lifecycle evidence must include PSS/RSS, thread, and FD samples so monotonic leaks are visible.

### D. Memory and CPU acceptance gates

Use Android Studio profiler plus `adb shell dumpsys meminfo`/CPU sampling while connecting to real peers.

- **Idle:** after a 15-minute warm-up, CPU must settle rather than show continuous wakeups or busy-loop behavior.
- **Churn:** run at least 10,000 connect/disconnect or failed-dial operations within configured limits; managed/native heap must return to a stable plateau after churn stops.
- **Messages:** sustained bounded application traffic must not grow the JNI/native/UI message queues beyond their documented caps.
- **Lifecycle:** 100 start/stop cycles must not produce monotonic native heap, thread, file-descriptor, socket, or callback growth.
- **Network changes:** 500 Wi-Fi/cellular availability changes (real or controlled test harness) must not leak callbacks/tasks or trigger unbounded redials.
- **Soak:** 6-hour active-traffic soak and 24-hour mostly-idle foreground-service soak; memory, thread count, FD count, and CPU must plateau and the API must remain responsive.

Record start, peak, post-churn, and end measurements as release evidence. A monotonic resource-growth trend is a blocker even if the app has not yet crashed.

### E. Security/hardening gates

- Fuzz the native bridge parsers/JSON/config and existing network fuzz targets with the Android workspace member enabled.
- Send malformed UTF-8/UTF-16, null/empty JNI strings, oversized payloads, malformed multiaddrs/PeerIds, and rapid concurrent API calls; JNI must return errors without panic/abort.
- Verify identity files remain inside app-private storage and cannot be redirected through path traversal/symlinks.
- Verify exported Android components: only the launcher Activity is exported; node service remains private.
- Confirm release signing, backup policy (`allowBackup=false`), minification rules, and supply-chain pinning before distribution.

## Release criterion

Android is production-qualified only when the root production gates, Android build/lint gates, functional matrix, lifecycle matrix, hostile bridge tests, and 24-hour resource soak are all green on the exact release commit. Until then, the desktop/core production status remains separate from Android APK qualification.
