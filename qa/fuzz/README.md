# Fuzzing

`qa/fuzz/` is intentionally separate from the stable workspace and is exercised by the scheduled `p2p-net-security-nightly` workflow with a pinned nightly toolchain and `cargo-fuzz` release.

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
cd qa/fuzz
cargo +nightly-2026-08-20 fuzz run app_message
```

The scheduled CI campaign builds every target and runs each for a bounded interval. Longer campaigns should be run before protocol/parser releases and whenever transport parsing changes.
