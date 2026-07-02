# Public bootstrap, rendezvous, and relay fallback

Public fallback is **enabled by default for normal app mode**. A fresh consumer node with no manual `bootstrap_peers`, no owned rendezvous peers, and no healthy peer cache may use public fallback to join the app discovery layer and may auto-dial app-namespace-discovered peers at the network layer.

Private-infrastructure-first is still supported, but it is now an advanced/operator mode: set `discovery.public_bootstrap.mode` to `disabled` and configure owned bootstrap, rendezvous, mediator, and relay peers explicitly.

## Config

Default consumer shape:

```json
{
  "discovery": {
    "rendezvous": {
      "client_enabled": true,
      "server_enabled": false
    },
    "dht": {
      "enabled": true,
      "announce": true,
      "discover": true,
      "discover_with_rendezvous_peers": true
    },
    "public_bootstrap": {
      "mode": "fallback_only",
      "bootstrap_seed_peers": [
        "/dnsaddr/bootstrap.libp2p.io/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
        "/dnsaddr/bootstrap.libp2p.io/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa",
        "/dnsaddr/bootstrap.libp2p.io/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
        "/dnsaddr/bootstrap.libp2p.io/p2p/QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt"
      ],
      "rendezvous_peers": [],
      "relay_peers": [],
      "auto_connect_discovered_peers": true
    }
  }
}
```

Advanced private-infrastructure-only shape:

```json
{
  "bootstrap_peers": [
    "/dnsaddr/bootstrap.example.net/p2p/<owned-bootstrap-peer-id>"
  ],
  "relay_peers": [
    "/dnsaddr/relay.example.net/p2p/<owned-relay-peer-id>"
  ],
  "discovery": {
    "rendezvous_peers": [
      "/dnsaddr/rendezvous.example.net/p2p/<owned-rendezvous-peer-id>"
    ],
    "public_bootstrap": {
      "mode": "disabled",
      "bootstrap_seed_peers": [],
      "rendezvous_peers": [],
      "relay_peers": [],
      "auto_connect_discovered_peers": false
    }
  }
}
```

`mode` values:

| Mode | Behavior |
|---|---|
| `disabled` | Never use public bootstrap, rendezvous, or relay candidates. Use this for private-infrastructure-first/operator mode. |
| `fallback_only` | Default. Use public bootstrap/rendezvous candidates only when owned/cached candidates are absent; use public relay candidates only when no operator/cached/rendezvous relays were selected. |
| `always` | Include public candidates after operator/cached candidates on every startup. |

Manual bootstrap peers remain optional power-user config. When `bootstrap_peers`, `discovery.bootstrap_seed_peers`, `discovery.rendezvous_peers`, or healthy peer-cache entries exist, `fallback_only` keeps those owned/cached candidates first.

## Bootstrap alone is not enough

Public bootstrap helps a node enter the wider routing layer, but it does not guarantee app-peer discovery or NAT traversal. The consumer default model has separate slots for bootstrap, app rendezvous, relay/mediator fallback, DHT provider discovery, and bounded auto-connect because each solves a different problem.

Applications that require two fresh installs on different networks to connect reliably should ship real public rendezvous and relay/mediator candidates or operate private infrastructure. Without those real endpoints, the repository can honestly provide public bootstrap and deterministic discovery logic, but it cannot promise NAT-to-NAT reachability.

## Auto-connect is not auto-trust

`auto_connect_discovered_peers = true` means app-namespace-discovered peers may be dialed at the network layer. It must not add the peer as a trusted contact. Contact trust remains an app-level action such as QR, join code, invite acceptance, or safety-number verification.

## Honest infrastructure defaults

The library default includes public bootstrap seeds. It does not ship a project-operated rendezvous or relay fleet, so the built-in `rendezvous_peers` and `relay_peers` lists are empty. App distributions that want reliable run-two-fresh-installs connectivity should publish real public rendezvous and relay/mediator DNSADDR entries and include them in:

- `discovery.public_bootstrap.rendezvous_peers`
- `discovery.public_bootstrap.relay_peers`

This avoids claiming rendezvous or relay service exists when this repo does not operate one. Public bootstrap still works as the consumer default discovery entry point; rendezvous and relay fallback become active automatically when real candidates are configured and policy allows them.

## Preference order

Startup discovery order is:

1. `bootstrap_peers`
2. `discovery.bootstrap_seed_peers`
3. `discovery.rendezvous_peers`
4. healthy peer-cache entries
5. `discovery.public_bootstrap.rendezvous_peers` when policy allows it
6. `discovery.public_bootstrap.bootstrap_seed_peers` when policy allows it

Relay selection order is:

1. `relay_peers`
2. healthy cached relay candidates
3. rendezvous relay candidates
4. `discovery.public_bootstrap.relay_peers` when policy allows it

When relay reservations are enabled, the runtime reserves through the selected relay set rather than only `relay_peers`, so real public relay/mediator candidates configured by an app distribution can provide NAT-to-NAT first-launch fallback. DCUtR remains enabled with relay-client capability so relayed connections can attempt direct upgrade while keeping relay fallback.

## Runtime safety

Built-in public fallback is best-effort. Invalid public multiaddrs still fail config validation, but DNS outages or unavailable public fallback peers should not prevent the app from starting. Observability fields show whether public fallback actually participated.

## Observability

Snapshots and metrics expose whether public fallback participated by category. Public rendezvous candidates are recorded as `public_rendezvous`, separate from operator-provided `rendezvous` sources, so app UIs can explain when a peer came from built-in public infrastructure rather than configured private infrastructure.

Snapshots and metrics expose whether public fallback participated:

- `public_fallback_mode`
- `public_fallback_used`
- `public_fallback_reason`
- `public_bootstrap_used` / `public_bootstrap_reason`
- `public_rendezvous_used` / `public_rendezvous_reason`
- `public_relay_used` / `public_relay_reason`
- `public_bootstrap_seed_count`
- `public_rendezvous_candidate_count`
- `public_relay_candidate_count`
- `p2p_public_fallback_used`
- `p2p_public_bootstrap_used`
- `p2p_public_rendezvous_used`
- `p2p_public_relay_used`
- `p2p_public_bootstrap_seed_count`
- `p2p_public_rendezvous_candidate_count`
- `p2p_public_relay_candidate_count`
