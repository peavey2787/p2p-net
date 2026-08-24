# Fuzzing

`qa/fuzz/` is intentionally separate from the stable workspace and is exercised by the scheduled `p2p-net-security-monthly` workflow with a pinned nightly toolchain and `cargo-fuzz` release.

Current targets cover the principal untrusted parsing surfaces:

- `heartbeat_wire`: compact heartbeat frame decoding and validation
- `node_config_json`: node configuration JSON
- `peer_cache_json`: persisted peer-cache JSON
- `app_message`: application-message envelope decoding/validation
- `dnsaddr_txt`: DNS-over-HTTPS TXT record decoding
- `peer_multiaddr`: arbitrary multiaddr parsing plus DNS/WebRTC classification
- `webrtc_stun`: WebRTC-direct STUN username/ufrag parsing in the patched transport

Local example:

```bash
rustup toolchain install nightly-2026-08-20 --profile minimal
cargo +nightly-2026-08-20 install cargo-fuzz --version 0.13.2 --locked
cargo +nightly-2026-08-20 fuzz build --fuzz-dir qa/fuzz
cargo +nightly-2026-08-20 fuzz run --fuzz-dir qa/fuzz app_message
```

The fuzz crate lives under `qa/fuzz/`, so every `cargo fuzz` invocation is run from the repository root with `--fuzz-dir qa/fuzz`. This is required because `cargo-fuzz` otherwise resolves its default harness as `<crate>/fuzz`, even if the shell working directory is changed to the nested fuzz package.

The scheduled CI campaign builds every target and runs each for a bounded interval. Longer campaigns should be run before protocol/parser releases and whenever transport parsing changes.
