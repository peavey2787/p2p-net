# Platform runtime abstraction

The platform runtime layer keeps one shared P2P core while moving platform-specific concerns to thin adapters.

The core has two small boundaries:

- `PlatformRuntime`: advisory platform facts used by environment/profile resolution.
- `NodeStorage`: persistence for identity keys, peer cache files, and small secrets.

The default `start_node(config)` path uses the desktop adapter. Embedders that are not plain desktop apps call:

```rust
start_node_with_platform(config, runtime, storage).await
```

where `runtime` is an `Arc<dyn PlatformRuntime>` and `storage` is an `Arc<dyn NodeStorage>`.

## Platform ownership

Platform policy is intentionally split rather than hidden behind one mixed mobile type:

```text
crates/platform/android.rs   Android runtime facts + app-private NodeStorage
crates/platform/desktop.rs   desktop runtime + filesystem NodeStorage
crates/platform/ios.rs       iOS runtime facts
crates/platform/memory.rs    test/embedder in-memory storage
crates/platform/traits.rs    shared traits only
```

UI, JNI, Android services, permissions, notifications, and Gradle code are not allowed in `crates/`. Android host code lives under `apps/android`; the desktop operator application lives under `apps/windows`.

## Desktop

`DesktopPlatformRuntime` implements both traits:

- runtime name: `desktop`
- storage kind: `desktop_fs`
- storage backend: standard filesystem
- default lifecycle: not background restricted
- listen capability: TCP and QUIC allowed

With `DesktopPlatformRuntime::default()`, relative paths resolve against the current process working directory. With `DesktopPlatformRuntime::with_data_dir(path)`, relative config paths are resolved under an app-owned data directory.

## Android

`AndroidPlatformRuntime` implements both `PlatformRuntime` and `NodeStorage`.

`AndroidPlatformRuntime::foreground_service(app_private_dir)` is the production native-app adapter:

- runtime name: `android_foreground_service`
- platform hint: Android
- TCP/QUIC listen capability enabled while the foreground service owns the node
- inbound public reachability left unknown so AutoNAT/relay discovery determines reality
- battery-sensitive hint retained
- background-restricted hint disabled only while the foreground service is active
- storage rooted under the app-private directory supplied by Kotlin
- relative storage keys only; absolute/path-traversal keys are rejected
- symlinked storage ancestors and final symlink targets are rejected
- secret identity material is create-only, non-regular-file reads are rejected, secret permissions are tightened on Unix/Android, and secret writes are synced

`AndroidPlatformRuntime::background_restricted(...)` remains available for planning/tests and reports no listen/inbound capability.

The full Android host is under `apps/android`. Its Rust `cdylib` bridge is the only Rust code allowed to perform FFI pointer handling; `crates/lib.rs` continues to forbid unsafe code. JNI conversion and Android lifecycle/UI concerns remain outside the shared core.

## iOS

`IosPlatformRuntime` owns iOS policy separately from Android. It defaults to battery-sensitive/background-restricted operation with inbound/listen capability disabled. Future Swift/Keychain integration belongs in an iOS app shell rather than Android or shared-core modules.

## Tests and adapters

`MemoryNodeStorage` is provided for tests and for embedders that want to keep persistence outside the core during early integration.

The storage-aware APIs are:

- `load_or_create_identity_key_with_storage(...)`
- `load_entries_with_storage(...)`
- `load_last_addrs_with_storage(...)`
- `record_seen_peer_addr_with_storage(...)`
- `record_peer_addr_failure_with_storage(...)`

Filesystem-oriented helpers remain desktop convenience wrappers around the storage-aware APIs.

## Architecture rule

Do not create separate networking implementations for Windows, Linux, macOS, Android, and iOS. Use one P2P core and keep platform-specific work limited to:

- storage
- lifecycle/background restrictions
- permissions
- app shell/UI
- packaging/bindings

`qa/tests/hygiene/platform_app_architecture.rs` enforces the desktop/Android layout, rejects Android UI/JNI concerns in `crates/`, and locks in the Android host's bounded low-overhead sampling design.

## Binding facade handoff

`crates/bindings/` is a JSON/enum-oriented facade for generic app shells. Platform runtimes and storage adapters remain the real boundary; binding hosts should use `prepare_binding_start_plan` to preview the resolved role/capabilities, then call `start_node_with_platform` with a durable `NodeStorage` implementation. The native Android app uses its focused C ABI/JNI bridge because it owns a long-lived foreground-service runtime. See `docs/impl/BINDINGS.md`.
