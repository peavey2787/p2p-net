# p2p-net-webrtc

`p2p-net-webrtc` is the hardened WebRTC-direct transport companion for `p2p-net`.
It is based on rust-libp2p `libp2p-webrtc` 0.9.0-alpha.1 and retains the same
public Rust library name (`libp2p_webrtc`) so the transport integration remains
compatible with rust-libp2p 0.56.

The fork carries p2p-net-specific production hardening for bounded half-open UDP
state, failed/cancelled connection cleanup, fail-safe connection close-on-drop,
and an explicit SRTP profile policy that excludes the known-problematic
AES-256-GCM path in the audited `webrtc` 0.12 generation.

Application developers normally do **not** depend on this crate directly. Add
`p2p-net = "0.1.0"` to the application's `Cargo.toml`; Cargo resolves this
companion automatically.
