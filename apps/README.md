# Application hosts

`apps/` contains platform application shells. It does **not** contain a second
networking implementation. All protocol, discovery, transport policy, security,
resource bounds, and observability behavior that can be shared stays in
`crates/`.

- `windows/` owns the desktop/operator `p2p_node` terminal application. Cargo
  still exposes it as `--example p2p_node` so existing developer commands stay
  compatible.
- `android/` owns the native Android foreground service, Rust C ABI bridge, JNI
  conversion shim, Compose UI, build tooling, and physical-device QA helpers.

Shared platform boundaries are equally explicit under `crates/platform/`:
`desktop.rs`, `android.rs`, and `ios.rs` contain only the narrow runtime/storage
policy required by the common core. Android lifecycle, JNI, and UI symbols must
never enter `crates/`; repository QA enforces that boundary.
