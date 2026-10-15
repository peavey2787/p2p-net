# TURN providers

p2p-net gets TURN credentials from a **provider**. Every provider ends up producing the
same small JSON bundle, so the node never needs provider-specific code paths:

```json
{
  "iceServers": [
    { "urls": ["stun:turn.example.org:3478"] },
    {
      "urls": ["turn:turn.example.org:3478?transport=udp", "turns:turn.example.org:443?transport=tcp"],
      "username": "short-lived-user",
      "credential": "short-lived-password"
    }
  ],
  "ttl": 3600
}
```

- `iceServers` uses the W3C `RTCIceServer` shape. `urls` may be a string or a list, and
  `iceServers` may be a single object or a list.
- `ttl` is how long the credentials stay valid, in seconds (60 to 172800). If it is
  missing, p2p-net assumes 3600.
- p2p-net keeps at most 8 entries per source, drops invalid ones, and refreshes at three
  quarters of the `ttl`.

## Where providers plug in

| Config | Runs where | Accepts |
| --- | --- | --- |
| `turn.client.cloudflare` / `turn.client.http` | browsers | `{"url": ...}` of an HTTPS endpoint returning the bundle above (no secrets: browser configs are public) |
| `turn.broker.provider` | broker nodes | `{"type": "http" \| "cloudflare" \| "shared_secret", ...}`; may hold long-lived secrets in `SecretRef`s |

| Broker `type` | Holds a long-lived secret | Use it for |
| --- | --- | --- |
| `http` | no | Any HTTPS endpoint returning the bundle above, such as the Cloudflare Worker in this folder |
| `cloudflare` | yes (API token) | Calling Cloudflare Realtime TURN directly from a p2p-net node |
| `shared_secret` | yes (TURN secret) | coturn `use-auth-secret` servers, or another p2p-net node's built-in TURN server |

Browsers get credentials from secret-holding providers through a broker node, which
hands them out over libp2p, or through an HTTP endpoint such as the Worker template.

## Templates

- [`cloudflare/`](cloudflare/README.md): a Cloudflare Worker that wraps Cloudflare
  Realtime TURN in the `http` contract.

## Adding a provider

There are two ways to add one. Pick the one that fits:

1. **Edge template (no Rust changes).** Add `providers/turn/<name>/` with a small
   function that calls the provider's API and returns the bundle above. Add
   `test/*.test.mjs` contract tests; validation runs every
   `providers/**/*.test.mjs` with `node --test`. Browsers then use it with
   `"turn": {"client": {"http": {"url": "..."}}}` and broker nodes with
   `{"type": "http", "url": "..."}`.
2. **Native provider (called directly by broker nodes).** Add
   `crates/connectivity/turn/provider/<name>.rs` with a config struct, `validate`,
   `issue(ttl)`, and the hosts its URLs point at, then add one variant to
   `TurnProviderConfig` (used by `turn.broker.provider`). Nothing else in the
   node changes: refresh, caching, broker distribution and browser ICE updates
   are shared. A browser-side source also needs a `TurnSource` variant and a
   `turn.client` field.

Keep secrets in `SecretRef` (`{"env": ...}` or `{"file": ...}`), never in source or
client configs.
