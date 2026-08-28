# Desktop operator application

`apps/windows/p2p_node.rs` owns the full-screen desktop/operator host and its
focused `p2p_node/view/` modules. Cargo continues to expose it as the
`p2p_node` example so existing developer commands remain stable:

```powershell
cargo run --release --features dashboard --example p2p_node -- --config p2p-node.json
```

The application shell owns terminal input/rendering, signal/console shutdown,
and operator presentation only. Networking, discovery, relay/DCUtR, storage
interfaces, resource bounds, security validation, and application messaging
remain in the shared `p2p-net` core under `crates/`.

Despite the `apps/windows` ownership name, the terminal host is also compiled by
Unix validation where supported. Native mobile application code must not be
added here or to the shared core.
