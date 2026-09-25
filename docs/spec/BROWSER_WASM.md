# Browser/WASM architecture

Browser parity means application API and protocol parity, not identical physical transports. `NodeHandle` remains the core API and `WasmNode` is a JS/WASM conversion and lifecycle facade only. No browser transport object is exposed to applications.

## Resolved role

`NodeProfile::Auto` on `PlatformKind::Wasm` resolves to the internal `NodeRole::WasmLite`. It enables Gossipsub, Kademlia client mode, relay client, rendezvous client, Identify, and Ping. It disables TCP/QUIC listeners, LAN UDP discovery, public-IP probing, AutoNAT/DCUtR, relay server, rendezvous server, and Kademlia server mode.

## Transport policy

The browser swarm is built with rust-libp2p's `wasm-bindgen` executor, `webrtc-websys`, `websocket-websys`, Noise/Yamux for WebSocket, and Circuit Relay client support. p2p-net does not create `RTCPeerConnection`, SDP, or ICE itself. WebRTC-direct is the primary direct browser-to-native path; secure WebSocket and Circuit Relay are supported fallback classes. WebTransport is intentionally not advertised until its native/browser interoperability gate is added and passed.

Browser connection planning filters unsupported addresses before a dial begins. Raw TCP and QUIC addresses are never selected merely because discovery returned them. DNS hostnames used for browser TLS transports are retained so the browser performs DNS/TLS/SNI. `/dnsaddr` continues to use bounded DoH and then applies the browser transport filter to returned records.

## Persistence

`BrowserNodeStorage` loads an IndexedDB journal asynchronously into the synchronous `NodeStorage` view before node startup. The identity is synchronously created in that view and then durably flushed before swarm construction. A periodic persistence task and lifecycle-triggered flushes persist later changes. Flushes are serialized and use write/flushed revision counters so concurrent mutations cannot be incorrectly marked durable.

The caller supplies the storage namespace. A profile ownership lock uses the Web Locks API when available and an IndexedDB lease when it is not, preventing simultaneous live nodes with one persisted PeerId.

## Management surface

`local_binding()` returns the local PeerId and only currently valid externally useful direct/relay listen addresses. `subscribe_events()` exposes only coarse `LocalBindingChanged`, `PeerConnected`, `PeerDisconnected`, `Online`, and `Offline` events. ICE candidates, relay internals, DCUtR internals, candidate pairs, and raw swarm events remain private.

## Message size

The application payload maximum remains 1 MiB on all targets. Large messages are internally fragmented/reassembled with bounded fragment count, per-peer and global memory ceilings, expiry, duplicate suppression, metadata consistency, and a full-payload BLAKE3 integrity check. Applications still perform one send/broadcast and receive one logical message.
