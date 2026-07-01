# Cross-platform bindings

The binding layer keeps one shared Rust P2P core and exposes binding-safe helpers for app shells. The goal is not to create separate Android, iOS, desktop, or web networking implementations. Each shell provides only lifecycle, storage, and UI adapters while `p2p-net` owns the node, transport policy, profiles, relay discovery, DCUtR, and event handling.

## Boundary

The shared core remains responsible for:

- `NodeConfig` loading and validation
- profile and capability resolution
- platform/environment-aware role selection
- libp2p transport and behaviour construction
- relay, mediator, rendezvous, DCUtR, and gossip policy
- snapshots and Prometheus-style metrics

Binding hosts are responsible for:

- showing UI
- collecting platform permissions
- choosing app-private storage locations
- supplying durable `NodeStorage` on mobile
- deciding when app lifecycle allows the node to run
- calling `start_node_with_platform(config, runtime, storage)`

## Binding facade

`src/bindings/mod.rs` contains JSON/enum-oriented helpers that are easy to wrap from UniFFI, a C ABI, Kotlin JNI, Swift FFI, Tauri, egui, or another host layer:

- `BindingTarget`
- `BindingRuntimeSpec`
- `BindingPlatformRuntime`
- `BindingStorageStrategy`
- `BindingStorageRequirement`
- `BindingStartPlan`
- `BindingSupportMatrix`
- `binding_support_matrix()`
- `node_config_from_json(raw)`
- `node_config_to_json(config)`
- `node_snapshot_to_json_string(snapshot)`
- `prepare_binding_start_plan(config_json, runtime_spec)`

The facade deliberately uses strings, enums, JSON, and existing config/snapshot types. It does not expose libp2p internals to app shells.

## Recommended shell model

```text
Desktop shell / Android shell / iOS shell
  ├─ owns UI and OS lifecycle
  ├─ builds BindingRuntimeSpec
  ├─ validates config through node_config_from_json
  ├─ previews role/capabilities through prepare_binding_start_plan
  ├─ supplies PlatformRuntime + NodeStorage
  └─ starts the same Rust core with start_node_with_platform
```

No shell should reimplement relay discovery, DCUtR policy, gossip validation, peer-cache semantics, identity loading, or profile resolution.

## Storage requirements

Desktop can use `DesktopPlatformRuntime` as both runtime and filesystem-backed storage.

Android/iOS/tablet shells should supply a durable `NodeStorage` adapter backed by app-private storage, encrypted storage, Keychain/Keystore, or a platform-owned encrypted container. `BindingStorageRequirement::ExternalPlatformStorage` means the host must provide this adapter before production startup.

`BindingStorageStrategy::MemoryTestingOnly` exists for tests, demos, and previews only. It must not be used for production identities because a restart changes the node identity and breaks stable peer identity expectations.

## Runtime expectations

Default target assumptions:

| Target | Runtime | Listen default | Background default | Production storage |
|---|---|---:|---:|---|
| Desktop | `binding_desktop` | yes | no | desktop filesystem or app data dir |
| Android | `binding_android` | no | yes | host-supplied durable storage |
| iOS/iPadOS | `binding_ios` | no | yes | host-supplied durable storage |
| WASM/WebView | `binding_wasm` | no | yes | host-supplied storage, usually restricted |

These defaults are advisory. Hosts can override listen/background hints in `BindingRuntimeSpec`, but mobile infrastructure roles should remain explicit and rare.

## Validation

`unit_tests/bindings.rs` checks that:

- all app shells use one shared Rust core
- Android/iOS resolve `profile = auto` to `mobile_lite`
- desktop defaults preserve listen capability
- JSON config/snapshot helpers work for binding UIs
- memory storage is flagged as testing-only

The test is registered in `Cargo.toml`, so the existing full validation scripts pick it up through `cargo test --workspace --locked -j 1`.

## CI

The repository CI now runs the existing stable validation script on Windows, Linux, and macOS. The Rust facade tests cover the target/runtime/storage planning logic for Android, iOS/iPadOS, desktop, and WASM/WebView. Once an app shell chooses a generator, add host-language package tests next to that shell, not by forking the P2P core.
