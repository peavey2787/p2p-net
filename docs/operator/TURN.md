# TURN

Browsers connect directly to each other with WebRTC after meeting over a libp2p relay.
On some networks (symmetric NAT, corporate firewalls, UDP-blocking Wi-Fi) the direct
path cannot be opened, and the browsers stay on the relay. TURN gives WebRTC a relay
it can always use, including TURN over TCP/TLS on port 443 when only web traffic gets
through.

p2p-net supports TURN in three parts, and you can use any combination of them:

| Part | Config | Who runs it |
| --- | --- | --- |
| **Client**: where browsers get TURN relay servers | `turn.client` | every browser node (automatic) |
| **Server**: a built-in TURN server | `turn.server` | any always-on native node (`profile: "turn"`) |
| **Broker**: hands out short-lived TURN credentials over libp2p | `turn.broker` | TURN nodes, or any node with a provider such as Cloudflare |

Nothing contacts a third-party TURN service unless you configure one.

TURN complements browser WebRTC; it does not replace it. `browser_webrtc` owns the
connection, the ICE policy (including TURN-only privacy mode), and STUN. TURN only
adds relay candidates to it. See [`BROWSER_WEBRTC.md`](BROWSER_WEBRTC.md) for every
combination (WebRTC without TURN, automatic TURN fallback, TURN only).

## Simple setups

### Run a TURN node

```json
{ "profile": "turn" }
```

That's the whole config (see `examples/turn-node.config.json`). The node:

1. finds its public IP with the public-IP probe;
2. starts a TURN server on UDP `3478` with relay ports `49160-49359`;
3. creates a random per-run secret and starts a broker that gives peers
   short-lived credentials over `/p2p-net/turn-credentials/1.0.0`.

Browsers connected to this node ask its broker automatically and add the TURN server to
their ICE configuration. Users don't configure anything.

Open these ports in the firewall: **UDP 3478** and **UDP 49160-49359**, plus the node's
normal libp2p listeners (browsers reach the broker over WebSocket or WebRTC-direct).

### Use Cloudflare TURN

Choose one of these setups:

- **Broker node**: one native node holds the Cloudflare API token and hands credentials
  to browsers. See `examples/turn-cloudflare-broker.config.json`.
- **Worker endpoint**: deploy `providers/turn/cloudflare` and point browsers at it with
  `"turn": {"client": {"cloudflare": {"url": ".../ice-servers"}}}`. See
  `examples/browser-turn-client.config.json`.

Cloudflare TURN also listens on TCP/TLS 443, so it gets through networks that block UDP.
Using it together with a built-in TURN node gives both fast UDP and a 443 fallback.
Setup details are in [`providers/turn/cloudflare/README.md`](../../providers/turn/cloudflare/README.md).

## Power-user settings

### `turn.server`

| Field | Default | Notes |
| --- | --- | --- |
| `enabled` | `false` | Switched on by `profile: "turn"`. |
| `bind_ip` | `0.0.0.0` | Local address for the listener and relay sockets. |
| `listen_port` | `3478` | UDP. Must be outside the relay range. |
| `public_ip` | probe result | Relay address handed to clients. Required when `public_ip_probe` is disabled. |
| `public_host` | none | DNS name used in advertised `turn:` URLs. Set it when browsers reach the node by name (`/dns4/.../wss`), so `same_host` clients accept it. |
| `realm` | `p2p-net` | TURN realm. |
| `shared_secret` | random per run | `{"env": ...}` or `{"file": ...}`. Set it to share the secret with coturn-style tooling or to keep credentials valid across restarts. |
| `relay_port_min` / `relay_port_max` | `49160` / `49359` | The range size caps concurrent allocations. |
| `allow_private_peers` | `false` | Off: the relay refuses to send to private, loopback and link-local addresses, so it cannot be used to reach your LAN. Turn it on only for LAN-only deployments. |

### `turn.broker`

| Field | Default | Notes |
| --- | --- | --- |
| `enabled` | `false` | Switched on by `profile: "turn"`. |
| `provider` | none | Upstream credentials (`cloudflare`, `http`, `shared_secret`). Combined with the built-in server when both are present. |
| `credential_ttl_secs` | `3600` | Lifetime requested from sources (60 to 172800). |
| `max_requests_per_peer_per_minute` | `6` | Per-peer rate limit; extra requests get `rate_limited`. |

The broker refreshes each source at three quarters of its lifetime, backs off
exponentially when a source fails, and answers from its cache. Long-lived secrets never
leave the node.

### `turn.client`

Sources (`libp2p` brokers, `self_hosted` servers, `cloudflare`, `http`), their
order, and trust settings are listed in
[`BROWSER_WEBRTC.md`](BROWSER_WEBRTC.md#turnclient-reference). Servers are merged in
source order. Duplicates are removed, expired credentials are dropped, and the browser
asks again before credentials expire.

## Security model

- **Media stays end-to-end encrypted.** WebRTC data channels are DTLS-encrypted
  between the two browsers. A TURN server only sees ciphertext and the two
  endpoints' addresses.
- **Credentials are short-lived and scoped to TURN.** Brokers hand out time-limited
  credentials over the authenticated, encrypted libp2p connection. Long-lived secrets
  (TURN shared secret, Cloudflare API token) never leave the node or Worker, and
  client configs that would ship one are rejected.
- **A broker serves every peer that connects to it**, within
  `max_requests_per_peer_per_minute`. All requests are answered from one cached
  credential set, so a flood of requests never multiplies upstream API calls. Anyone
  who can reach the node can still use your TURN capacity. Limit that capacity with
  the relay port range, `connection_limits`, and Cloudflare usage alerts, or run the
  broker only on private infrastructure.
- **Browsers do not trust arbitrary TURN hosts.** Under `same_host`, a broker can only
  point browsers at its own host or at a trusted provider host, so a malicious peer
  cannot steer browser traffic to third parties.
- **The built-in relay cannot reach your LAN.** Relays to private, loopback and
  link-local addresses are refused unless `allow_private_peers` is on.

## Observability

`NodeSnapshot.turn` reports:

- **Browsers**: `ice_servers`, `turn_servers`, `ice_transport_policy`, `stun_enabled`,
  `client_sources`, `discovery`, `client_endpoints_active`, `client_endpoint_errors`,
  `broker_peers`, `credential_requests_sent`, and `credential_servers_accepted` /
  `rejected`.
- **TURN server**: `server_state` (`disabled`, `waiting_for_public_ip`, `running`,
  `failed`), `server_listen`, `server_urls`, `server_allocations` /
  `server_allocation_capacity`, and `server_error`.
- **Broker**: `broker_sources`, `broker_ready_sources`, `broker_granted`,
  `broker_rate_limited`, `broker_unavailable`, `broker_source_errors`, and
  `broker_last_error`.

Pulse lines: `TURN_SERVER_STARTED`, `TURN_SERVER_FAILED`, `TURN_BROKER_REFRESHED`,
`TURN_BROKER_SOURCE_FAILED`, `TURN_CREDENTIALS_GRANTED`, `TURN_CREDENTIALS_RECEIVED`,
`TURN_PROVIDER_REFRESHED` and `TURN_PROVIDER_FAILED`.

## Troubleshooting

- **`server_state: failed`, "no public IP found"**: the probe could not reach its
  endpoints. Set `turn.server.public_ip`.
- **Browsers show `turn_servers: 0` while a TURN node is connected**: the browser
  probably dialed the node by DNS name while the node advertises its IP. Set
  `turn.server.public_host`, or add the host to `turn.client.trusted_hosts`.
- **Allocations work but no media flows**: the relay port range is not open in the
  firewall.
- **`broker_source_errors` growing**: check `broker_last_error`, for example a missing
  `CLOUDFLARE_TURN_API_TOKEN` environment variable or an expired token.
