# Browser WebRTC direct upgrade

Two browsers cannot dial each other directly: neither can listen on a public
address. p2p-net connects them through a Circuit Relay and then upgrades the
pair to a direct libp2p WebRTC (`/webrtc`, browser-to-browser) connection,
using the relayed connection only for signaling. The relay stays the fallback
whenever the upgrade does not succeed. Native peers are unaffected: they keep
using DCUtR.

## Path lifecycle

1. **Relay connect.** Each browser reserves a slot on a relay (`/p2p-circuit`
   listen address) and dials peers through it. Pulse: `RELAY_CONNECT`.
2. **Signaling.** On a relayed connection between two browsers, the
   `p2p-net-webrtc-websys` signaling behaviour opens a
   `/webrtc-signaling/0.0.1` stream over that connection and exchanges the SDP
   offer/answer and ICE candidates. The dialer of the relayed connection is the
   initiator. Pulse: `WEBRTC_SIGNALING peer=… initiator=…`.
3. **ICE check.** The browsers run ICE with the configured servers. Pulse:
   `ICE_CHECK peer=… outcome=connected|failed|timeout` (the failure forms
   include the final `iceConnectionState`).
4. **Upgrade.** On success the new connection is authenticated (Noise over
   the data channel) and reaches the swarm on each side, which counts it as
   `DIRECT_UPGRADE_SUCCESS`. The relayed connection stays the path until the
   remote proves it has the direct connection too: its Identify arriving over
   that connection. Only then does p2p-net close the peer's relayed
   connections, moving application traffic to the direct path. Pulse:
   `PATH_MIGRATED_TO_DIRECT peer=… relayed_closed=N`. The relay reservation
   itself is kept. (Closing as soon as the local side is up would cut the
   remote's signaling while it is still completing its side of the upgrade.)
5. **Fallback.** On failure: `DIRECT_UPGRADE_FAILED peer=… error=…`, and, when
   a relayed connection to the peer exists, `RELAY_FALLBACK peer=…`. Traffic
   keeps flowing over the relay. A failed `RtcPeerConnection` is closed.

A direct `/webrtc-direct` dial to a listening native peer reports
`DIRECT_DIAL peer=… addr=…` and never involves the relay.

Pulses are lifecycle events only, never per packet. The counters are in the
runtime snapshot (`direct_upgrade`: `upgrades_succeeded`, `upgrades_failed`,
`paths_migrated`, `relayed_peers`) and in Prometheus
(`p2p_direct_upgrade_successes`, `p2p_direct_upgrade_failures`,
`p2p_direct_upgrade_paths_migrated`, `p2p_direct_upgrade_relayed_peers`).

## No third-party infrastructure by default

The upgrade depends on nothing but the relay p2p-net already uses: no LAN
discovery, no mDNS, and no public STUN/TURN. With no ICE servers configured,
browsers offer their host candidates only. That connects browsers on the same
host or network; across NATs it usually fails, and the pair stays on the
relay. Operators who want direct paths across NATs configure their own
STUN/TURN through `browser_webrtc`:

```json
{
  "browser_webrtc": {
    "ice_servers": ["stun:stun.example.org:3478"],
    "ice_transport_policy": "all"
  }
}
```

`ice_transport_policy: "relay"` restricts ICE to TURN candidates (hides local
addresses); without TURN servers it disables direct upgrades entirely.

## Capability flags

`TransportCapabilities` reports exactly what runs. Browsers set
`browser_webrtc` and `relay_assisted_upgrade` and list `webrtc-browser`,
`webrtc-direct` (dial) and `webtransport` (dial) in `active_transports`, never
`tcp`, `quic` or `dcutr`. Native nodes never claim `browser_webrtc`.

## Companion fixes this relies on

- `p2p-net-webrtc-websys` (libp2p-webrtc-websys 0.5.0 plus rust-libp2p PR
  #5978): browser `/webrtc` signaling, with bounded signaling messages, failure
  reporting, `CloseOnFailure`, relay-only ICE, and the `ICE_CHECK` outcome.
- `p2p-net-webrtc`: native WebRTC streams keep their full 16 KiB read
  capacity when the data channel is cloned, and never send a data channel
  message over 16 KiB (`FullFrameChannel`). Before these fixes, browser frames
  above 8 KiB (`ErrShortBuffer`) and coalesced outbound frames ("outbound
  packet larger than maximum message size") killed relayed circuits.
- `p2p-net-gossipsub`: a connection that is not a peer's first has no
  gossipsub outbound stream, and its handler was never woken by queued
  messages. After `PATH_MIGRATED_TO_DIRECT` closed the relayed (first)
  connection, messages sat in the queue until they expired. The handler now
  registers a waker on the queue.
- `p2p-net-relay`: a relayed dial issued while the dial to that relay is still
  in flight now waits for the relay connection instead of being cancelled.

## Validation

`qa/browser/run-direct-upgrade.cjs` (the `wasm` stage of
`run-full-validation`) runs real Chromium pages against the
`qa_browser_relay` example, a loopback relay with native peers:

- `capabilities`: the browser advertises only the transports it runs.
- `success`: browser↔browser relay → signaling → ICE → migration. The relay's
  metered transit is frozen while 16 messages of 4 KiB flow both ways.
- `failure`: forced ICE failure (relay-only ICE without TURN). The pair falls
  back and its messages measurably cross the relay.
- `native-direct`: browser → native over `/webrtc-direct`, with no relay
  transit.
- `native-relay`: browser → native relay-only peer (no signaling support)
  works through the relay fallback.

Native↔native DCUtR behaviour is covered by the existing relay/DCUtR tests.
