# Platform runtime abstraction

Phase 9 keeps one shared P2P core while moving platform-specific concerns to thin adapters.

The core now has two small boundaries:

- `PlatformRuntime`: advisory platform facts used by environment/profile resolution.
- `NodeStorage`: persistence for identity keys, peer cache files, and small secrets.

The default `start_node(config)` path still uses the desktop adapter and preserves the previous behavior where `identity_key_path` and `discovery.peer_cache_path` are interpreted as filesystem paths.

Embedders that are not plain desktop apps should call:

```rust
start_node_with_platform(config, runtime, storage).await
```

where `runtime` is an `Arc<dyn PlatformRuntime>` and `storage` is an `Arc<dyn NodeStorage>`.

## Desktop

`DesktopPlatformRuntime` implements both traits:

- runtime name: `desktop`
- storage kind: `desktop_fs`
- storage backend: standard filesystem
- default lifecycle: not background restricted
- listen capability: TCP and QUIC allowed

With `DesktopPlatformRuntime::default()`, relative paths behave exactly like the old implementation.

With `DesktopPlatformRuntime::with_data_dir(path)`, relative config paths are resolved under an app-owned data directory.

## Mobile

`MobilePlatformRuntime` is a policy hook for Android/iOS shells:

- runtime name: `mobile`
- Android/iOS platform hint
- background-restricted by default
- battery-sensitive by default
- inbound listen disabled by default

It intentionally does not implement storage. Android/iOS shells should provide a platform-specific `NodeStorage` implementation that writes into app-private storage, an encrypted container, Android Keystore-backed data, iOS Keychain-backed data, or another app-owned backend.

## Tests and adapters

`MemoryNodeStorage` is provided for tests and for embedders that want to keep persistence outside the core during early integration.

The storage-aware APIs are:

- `load_or_create_identity_key_with_storage(...)`
- `load_entries_with_storage(...)`
- `load_last_addrs_with_storage(...)`
- `record_seen_peer_addr_with_storage(...)`
- `record_peer_addr_failure_with_storage(...)`

The old filesystem helpers remain as compatibility wrappers around the desktop adapter.

## Architecture rule

Do not create separate networking implementations for Windows, Linux, macOS, Android, and iOS.

Use one P2P core and keep platform-specific work limited to:

- storage
- lifecycle/background restrictions
- permissions
- app shell/UI
- packaging/bindings

## Binding facade handoff

Phase 10 adds `src/bindings/` as a JSON/enum-oriented facade for app shells. Platform runtimes and storage adapters remain the real boundary; binding hosts should use `prepare_binding_start_plan` to preview the resolved role/capabilities, then call `start_node_with_platform` with a durable `NodeStorage` implementation. See `docs/BINDINGS.md`.
