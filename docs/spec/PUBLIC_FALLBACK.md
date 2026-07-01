# Public bootstrap and relay fallback

Public fallback is an opt-in recovery mechanism. Nodes always prefer operator-owned bootstrap, rendezvous, mediator, and relay infrastructure first. Public resources are considered only when the operator explicitly configures them.

## Config

```json
{
  "discovery": {
    "public_bootstrap": {
      "mode": "fallback_only",
      "bootstrap_seed_peers": [
        "/dnsaddr/bootstrap.libp2p.io/p2p/<peer-id>"
      ],
      "relay_peers": [
        "/ip4/203.0.113.10/tcp/4001/p2p/<relay-peer-id>"
      ]
    }
  }
}
```

`mode` values:

| Mode | Behavior |
|---|---|
| `disabled` | Never use public bootstrap or relay candidates. This is the default. |
| `fallback_only` | Use public bootstrap seeds only when there are no operator/cached startup candidates; use public relay candidates only when no operator/cached/rendezvous relays were selected. |
| `always` | Include public candidates after operator/cached candidates. |

The crate does not hide hard-coded public seed defaults in the runtime. Operators must decide which public bootstrap or relay peers they trust enough to configure.

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

## Current boundary

The current policy is evaluated at startup from configured and cached candidates. A later runtime-health layer can add delayed fallback when operator-owned candidates exist but every dial attempt fails.

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
