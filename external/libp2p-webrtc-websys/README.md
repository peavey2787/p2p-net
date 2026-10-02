# p2p-net-webrtc-websys

`p2p-net-webrtc-websys` is the browser WebRTC companion for `p2p-net`. It is
rust-libp2p `libp2p-webrtc-websys` 0.5.0 (MIT) plus browser-to-browser
`/webrtc` support from the open rust-libp2p pull request 5978, and it keeps the
public Rust library name `libp2p_webrtc_websys`.

- `Transport` dials `/webrtc-direct` servers, as upstream 0.5.0 does.
- `browser::Transport` + `browser::Behaviour` implement the libp2p WebRTC
  private-to-private protocol: two browsers that share an authenticated
  relayed connection exchange SDP offer/answer and ICE candidates over a
  `/webrtc-signaling/0.0.1` stream, then open a direct `RtcPeerConnection`.
  The DTLS fingerprints travel inside the SDP over the authenticated relayed
  connection, which binds the direct connection to the peer's identity.

p2p-net changes on top of the pull request:

- The signaling message uses prost (byte-identical to the spec's protobuf) and
  is bounded to 64 KiB per message; timers use `futures-timer`; dependencies
  used only by the pull request's examples (relay, identify, ping, yamux,
  websocket) are not pulled in.
- The initiator is the dialer of the relayed connection (the spec's
  convention), so both sides never offer at once.
- The browser transport no longer drops an incoming direct connection when no
  listener is registered, and the behaviour wakes itself on its retry timer.
- Success is reported as soon as the peer connection is connected, without
  waiting for ICE gathering to complete.
- Every failure reaches the behaviour as `SignalingEvent::WebRTCConnectionError`
  (a failed `RtcPeerConnection` is closed); `SignalingStarted` marks the start of
  an attempt; a failed connectivity check is a typed `Error::IceCheck` with
  `IceCheckOutcome::{Failed, TimedOut}` and the final ICE state.
- `browser::SignalingConfig::with_relay_only_ice` restricts ICE to TURN candidates.

ICE servers are optional configuration: nothing here contacts a public STUN or
TURN service unless the application configures one.

Application developers normally do **not** depend on this crate directly. Add
`p2p-net = "0.1.0"` to the application's `Cargo.toml`; Cargo resolves this
companion automatically.
