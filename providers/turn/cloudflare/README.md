# Cloudflare Realtime TURN for p2p-net

This template turns [Cloudflare Realtime TURN](https://developers.cloudflare.com/realtime/turn/)
into the p2p-net [provider contract](../README.md). You can use Cloudflare TURN in two
ways.

## Option A: Worker endpoint (browsers fetch credentials directly)

The Worker keeps the TURN key's API token as a secret and gives browsers only
short-lived credentials.

1. In the Cloudflare dashboard, open **Realtime > TURN Server** and create a TURN key.
   Note its **key ID** and **API token**.
2. In this folder, set `TURN_KEY_ID` and `ALLOWED_ORIGINS` in `wrangler.toml`, then run:

   ```bash
   npx wrangler secret put TURN_KEY_API_TOKEN
   ```

   ```bash
   npx wrangler deploy
   ```

3. Point the browser config at the Worker:

   ```json
   {
     "turn": {
       "client": {
         "cloudflare": { "url": "https://p2p-net-turn.<account>.workers.dev/ice-servers" }
       }
     }
   }
   ```

   Whether browsers also try direct paths or use TURN only is set separately in
   `browser_webrtc` (see `docs/operator/BROWSER_WEBRTC.md`).

The Worker:

- answers `GET /ice-servers` with `{"iceServers": [...], "ttl": N}`;
- removes port-53 URLs, which browsers block;
- allows only the origins in `ALLOWED_ORIGINS` (requests with no `Origin` header,
  such as a p2p-net broker node, are allowed);
- applies the optional `RATE_LIMITER` binding (20 requests per IP per minute by
  default). Remove the `[[ratelimits]]` block if your plan does not include it.

Run the contract tests (no network needed) with `node --test "test/*.test.mjs"`.

## Option B: p2p-net broker node (no Worker)

A native p2p-net node can call Cloudflare itself and hand credentials to browsers over
libp2p. Browsers then need no TURN configuration at all:

```json
{
  "turn": {
    "broker": {
      "enabled": true,
      "provider": {
        "type": "cloudflare",
        "key_id": "your-cloudflare-turn-key-id",
        "api_token": { "env": "CLOUDFLARE_TURN_API_TOKEN" }
      }
    }
  }
}
```

See `examples/turn-cloudflare-broker.config.json`. Browsers accept Cloudflare's
`turn.cloudflare.com` servers from any broker under the default `same_host` policy.

Usage is billed to the Cloudflare account that owns the TURN key. Watch it in the
dashboard and keep `ALLOWED_ORIGINS` and the rate limit tight.
