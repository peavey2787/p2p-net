# Browser smoke app

This is the reference zero-networking-logic browser consumer of `p2p-net`. All WebRTC, WebSocket, relay, discovery, framing, persistence, and reconnect behavior stays inside the Rust/WASM package.

Build the package from the repository root:

```sh
wasm-pack build --target web --dev --out-dir apps/browser/pkg
```

Then serve the repository through an HTTP server and open `apps/browser/`. The page calls only `WasmNode`; it does not use `RTCPeerConnection`, SDP, ICE, STUN, WebSocket, or libp2p directly.
