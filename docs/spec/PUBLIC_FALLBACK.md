# Public bootstrap and relay fallback

Public fallback is **enabled by default for normal app mode**. A fresh consumer node with no manual `bootstrap_peers`, no owned rendezvous peers, and no healthy peer cache may use the built-in public bootstrap list to join the wider discovery layer.

Private-infrastructure-first is still supported, but it is now an advanced/operator mode: set `discovery.public_bootstrap.mode` to `disabled` and configure owned bootstrap, rendezvous, mediator, and relay peers explicitly.

## Config

Default consumer shape:

```json
{
  "discovery": {
    "public_bootstrap": {
      "mode": "fallback_only",
      "bootstrap_seed_peers": [
        "/dnsaddr/bootstrap.libp2p.io/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
        "/dnsaddr/bootstrap.libp2p.io/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa",
        "/dnsaddr/bootstrap.libp2p.io/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
        "/dnsaddr/bootstrap.libp2p.io/p2p/QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt"
      ],
      "relay_peers": []
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
      "relay_peers": []
    }
  }
}
```

`mode` values:

| Mode | Behavior |
|---|---|
| `disabled` | Never use public bootstrap or relay candidates. Use this for private-infrastructure-first/operator mode. |
| `fallback_only` | Default. Use public bootstrap seeds only when there are no operator/cached startup candidates; use public relay candidates only when no operator/cached/rendezvous relays were selected. |
| `always` | Include public candidates after operator/cached candidates on every startup. |

Manual bootstrap peers remain optional power-user config. When `bootstrap_peers`, `discovery.bootstrap_seed_peers`, or healthy peer-cache entries exist, `fallback_only` keeps those owned/cached candidates first and does not use public bootstrap unless the owned/cached startup set is empty.

## Public relay defaults

The library default includes public bootstrap seeds. It does not ship a project-operated relay fleet, so the built-in `relay_peers` list is empty. App distributions that want default NAT-to-NAT relay connectivity should publish public relay/mediator DNSADDR entries and include them in `discovery.public_bootstrap.relay_peers`, or operate private relays and put them in `relay_peers`.

This avoids claiming a relay exists when this repo does not operate one. Public bootstrap still works as the consumer default discovery entry point; relay fallback becomes active automatically when public relay candidates are configured and policy allows them.

## Preference order

Startup discovery order is:

1. `bootstrap_peers`
2. `discovery.bootstrap_seed_peers`
3. `discovery.rendezvous_peers`
4. healthy peer-cache entries
5. `discovery.public_bootstrap.bootstrap_seed_peers` when policy allows it

Relay selection order is:

1. `relay_peers`
2. healthy cached relay candidates
3. rendezvous relay candidates
4. `discovery.public_bootstrap.relay_peers` when policy allows it

## Runtime safety

Built-in public fallback is best-effort. Invalid public multiaddrs still fail config validation, but DNS outages or unavailable public fallback peers should not prevent the app from starting. Observability fields show whether public fallback actually participated.

## Observability

Snapshots and metrics expose whether public fallback participated:

- `public_fallback_mode`
- `public_fallback_used`
- `public_fallback_reason`
- `public_bootstrap_seed_count`
- `public_relay_candidate_count`
- `p2p_public_fallback_used`
- `p2p_public_bootstrap_seed_count`
- `p2p_public_relay_candidate_count`
