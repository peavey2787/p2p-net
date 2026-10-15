# Choosing a role

Pick one word for `profile` and p2p-net configures everything else. You don't need to
know anything about networking to use the defaults. Every setting the role chooses
can still be overridden in the config file (see "Fine-tuning" below).

| I want to... | `profile` | What it does |
| --- | --- | --- |
| Use an app normally | `auto` (default) | Picks the right role for this device: `full` on a reachable desktop, `lite` behind strict NAT, `mobile_lite` on phones, and `wasm_lite` in browsers. |
| Run a reliable desktop or server peer | `full` | Listens for peers and takes part in the DHT. |
| Save battery and bandwidth | `lite` / `mobile_lite` | Connects out only and relies on relays. Never serves others. |
| Help peers behind NAT reach each other | `relay` | Forwards encrypted libp2p traffic for peers that cannot connect directly. |
| Help phones punch through NAT | `mediator` | A relay that also coordinates direct-connection upgrades (DCUtR). |
| Help browsers connect when WebRTC is blocked | `turn` | Runs a TURN server and hands out short-lived TURN credentials to peers. See [TURN.md](TURN.md). |
| Help apps find each other | `rendezvous` | Serves rendezvous registrations and lookups. |
| Be a stable entry point into the network | `bootstrap` | Keeps a stable address for new nodes to join through. |

Examples:

```json
{ "profile": "turn" }
```

```json
{ "profile": "relay" }
```

The desktop node takes the same word on the command line, on its own or on top of a
config file:

```bash
cargo run --release --features dashboard --example p2p_node -- --profile turn
```

## Combining roles

A node has one `profile`, but you can switch on extra services next to it:

```json
{
  "profile": "turn",
  "relay": { "enabled": true }
}
```

This node is a TURN server and a libp2p relay, which is a good general helper for
browser-heavy apps.

## Rules the resolver enforces

- Lite, mobile and browser roles never run servers (relay, rendezvous, TURN). A config
  that asks for one fails with a clear error instead of quietly draining a phone.
- Background-restricted environments cannot run relay or TURN servers.
- Services that need a reachable address (relay, TURN) refuse to start without one,
  and report why in the snapshot and pulse stream.

## Fine-tuning

Power users can change any setting a role picked. The full field list is in the
README's config section; the main groups are:

- `listen_addresses`, `listeners`, `external_addresses`: where and how the node is reachable.
- `relay.*` and `mediator.*`: relay capacity, ACLs and service windows.
- `browser_webrtc`: browser-to-browser WebRTC, ICE policy, direct connections and STUN
  ([BROWSER_WEBRTC.md](BROWSER_WEBRTC.md)).
- `turn.client`, `turn.server`, `turn.broker`: TURN relay sources for browsers, the
  built-in TURN server and credential distribution ([TURN.md](TURN.md)).
- `discovery.*`: bootstrap, rendezvous, DHT and LAN discovery.
- `connection_limits`, `message_security`, `dcutr`: safety limits and upgrade policy.

The profile is applied first, then your explicit settings, then the capability resolver
checks the result. `NodeConfig::try_resolved()` (and the snapshot's
`active_transports`) shows exactly what will run.
