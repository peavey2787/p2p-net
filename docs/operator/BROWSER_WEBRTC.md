# Browser WebRTC and TURN

Browsers running p2p-net first meet over a libp2p Circuit Relay connection. They then
try to open a direct WebRTC connection to each other and move onto it. Two config
sections control that, and each works on its own:

| Section | Owns |
| --- | --- |
| `browser_webrtc` | Whether browsers try WebRTC at all, the ICE policy, direct connections, and STUN. |
| `turn.client` | TURN relay servers and their credentials: where they come from, in what order, and how many. |

WebRTC decides *how* to connect. TURN only adds relay candidates. TURN can never
override a WebRTC policy; for example, it cannot re-enable direct connections that
`browser_webrtc` turned off.

## Automatic mode (the default)

With no settings at all (`{}`), browsers:

1. try WebRTC with every candidate type (`ice_transport_policy: "all"`);
2. use TURN servers that p2p-net TURN nodes on the network offer (`turn.client`
   `auto_discover`);
3. contact no third-party STUN or TURN service;
4. stay on the Circuit Relay connection whenever no direct or TURN path works.

So the connection picks the best working path by itself: direct, then TURN, then
Circuit Relay. Add servers to give it more paths.

## Common setups

### WebRTC without TURN

Direct connections only, with nothing to run or pay for. Add a STUN server so
browsers behind NAT can find their public addresses (`examples/browser-webrtc-direct.config.json`):

```json
{
  "browser_webrtc": { "stun": { "servers": ["stun:stun.cloudflare.com:3478"] } },
  "turn": { "client": { "enabled": false } }
}
```

Leave out `stun.servers` to use local candidates only; that works on the same
network.

### WebRTC with automatic TURN fallback

Browsers try direct paths first. If none works, ICE uses TURN on its own
(`examples/browser-turn-client.config.json`):

```json
{
  "browser_webrtc": {
    "ice_transport_policy": "all",
    "direct_connections": true,
    "stun": { "enabled": true, "servers": ["stun:stun.cloudflare.com:3478"] }
  },
  "turn": {
    "client": {
      "enabled": true,
      "auto_discover": true,
      "providers": ["libp2p", "self_hosted", "cloudflare"],
      "servers": [{ "urls": "turn:turn.example.org:3478", "username": "u", "credential": "p" }],
      "cloudflare": { "url": "https://p2p-net-turn.YOUR_ACCOUNT.workers.dev/ice-servers" },
      "max_servers": 4
    }
  }
}
```

### TURN only, for privacy

`ice_transport_policy: "relay"` makes WebRTC use TURN candidates only. The other
browser never sees your IP address, and p2p-net also removes every STUN URL, so no
STUN server sees it either. The TURN operator can still see both endpoints and
traffic metadata; the data itself stays DTLS-encrypted end to end. See
`examples/browser-turn-only-privacy.config.json`:

```json
{
  "browser_webrtc": { "ice_transport_policy": "relay", "stun": { "enabled": false } },
  "turn": {
    "client": { "cloudflare": { "url": "https://p2p-net-turn.YOUR_ACCOUNT.workers.dev/ice-servers" } }
  }
}
```

`"direct_connections": false` means the same thing. If no TURN server is
available, the pair simply stays on Circuit Relay. It never falls back to a direct
path.

### No browser WebRTC

`"browser_webrtc": { "enabled": false }` keeps browsers on Circuit Relay and the
other transports. It also turns off TURN discovery, since TURN only feeds WebRTC.

## `browser_webrtc` reference

| Field | Default | Notes |
| --- | --- | --- |
| `enabled` | `true` | Relay-assisted browser-to-browser WebRTC upgrade. |
| `ice_transport_policy` | unset (`all`) | `all` or `relay`. |
| `direct_connections` | unset (`true`) | `false` is the same as `ice_transport_policy: "relay"`. |
| `stun.enabled` | `true` | `false` removes every STUN URL, including ones TURN sources offer. |
| `stun.servers` | `[]` | `stun:`/`stuns:` URLs, at most 4. |
| `ice_servers` | `[]` | The original list of `stun:`/`turn:` URLs or `RTCIceServer` objects. Still supported: STUN entries are added to `stun.servers`, and TURN entries are added to `turn.client.servers`. |

## `turn.client` reference

| Field | Default | Notes |
| --- | --- | --- |
| `enabled` | `true` | `false` means no TURN at all. |
| `auto_discover` | `true` | The `libp2p` source: TURN servers advertised by p2p-net broker peers (`profile: "turn"` nodes). This is not Circuit Relay. |
| `providers` | unset | The exact source order. Unset: every configured source, in the order `self_hosted`, `libp2p`, `http`, `cloudflare`. |
| `servers` | `[]` | The `self_hosted` source: static TURN entries with `username` and `credential`. |
| `cloudflare` | none | `{"url": ...}` of a Cloudflare credential endpoint such as [`providers/turn/cloudflare`](../../providers/turn/cloudflare/README.md). |
| `http` | none | `{"url": ...}` of any endpoint that follows the [provider contract](../../providers/turn/README.md). |
| `discovery_trust` | `same_host` | `same_host`: accept a broker's TURN servers only on the host the browser dialed it at, on Cloudflare, or on a `trusted_hosts` entry. `any`: accept every broker's servers. |
| `trusted_hosts` | `[]` | Extra TURN hosts accepted from brokers. |
| `max_servers` | `4` | TURN entries per peer connection (1 to 8). |
| `credential_ttl_secs` | `3600` | Lifetime requested from endpoints and brokers. |

Browser configs are public, so `turn.client` endpoints cannot carry a
`bearer_token`. Keep API tokens in a Worker or on a broker node (`turn.broker`).

## Precedence and conflicts

- **ICE policy:** `relay` wins. If either `ice_transport_policy: "relay"` or
  `direct_connections: false` is set, ICE is relay-only. Setting both fields to
  values that disagree is rejected.
- **TURN sources:** `turn.client.enabled: false` turns every source off. An explicit
  `providers` list is used exactly as written. Otherwise sources are picked
  automatically.
- **Nothing is ignored silently.** p2p-net refuses to start when settings would have
  no effect or contradict each other. Examples:
  - STUN servers listed while `stun.enabled` is false;
  - WebRTC or TURN client settings while `browser_webrtc.enabled` is false;
  - TURN sources configured while `turn.client.enabled` is false;
  - a `providers` list that names an unconfigured source or leaves out a configured one;
  - `libp2p` listed while `auto_discover` is false;
  - more than 8 ICE servers in total.

## Snapshot

`NodeSnapshot.turn` shows what a browser is using:

- `ice_servers` and `turn_servers`: the composed list.
- `ice_transport_policy` and `stun_enabled`.
- `client_sources`, `discovery`, `client_endpoints_active` and
  `client_endpoint_errors`.
