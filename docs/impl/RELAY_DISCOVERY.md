# Relay discovery and selection

A first-class relay discovery policy lets lite and mobile-lite nodes avoid hard-coding every relay forever.

The policy lives at:

```json
"discovery": {
  "relay_discovery": {
    "enabled": true,
    "use_configured_relays": true,
    "use_cached_relays": true,
    "use_rendezvous_relays": true,
    "min_reservations": 1,
    "max_reservations": 3,
    "prefer_configured_relays": true,
    "replace_failed_relays": true
  }
}
```

## Sources

Relay candidates are selected through one deterministic pipeline:

1. **Configured relays** from `relay_peers`. These are operator-pinned and preferred by default.
2. **Cached relays** from the identity-bound peer cache. Expired, failed, bare, unspecified, and `/p2p-circuit` addresses are filtered out before selection.
3. **Rendezvous candidates** from configured rendezvous infrastructure. This lets deployments publish likely relay/mediator nodes through their rendezvous/bootstrap layer.
4. **Public fallback candidates** from `discovery.public_bootstrap.relay_peers` when public fallback policy allows them.

All candidates must be identity-bound `/p2p/<PeerId>` multiaddrs with a reachable transport. Already-relayed `/p2p-circuit` addresses are not used as relay reservation targets.

## Selection rules

- Candidates are deduplicated by relay peer ID.
- Configured relays are preferred when `prefer_configured_relays = true`.
- Selection stops at `max_reservations`.
- A warning pulse is emitted when fewer than `min_reservations` candidates are available.
- Public fallback candidates are ordered after operator-owned/cached/rendezvous candidates.
- Setting `enabled = false` means only configured `relay_peers` are considered.

## Runtime behavior

Startup reserves through the selected relay set instead of only raw `relay_peers`. Automatic cached/rendezvous relay discovery is enabled for lite/mobile-lite roles; full/infrastructure nodes also enable relay discovery when explicit configured relays or public fallback relay candidates are available.

When `reserve_configured_relays = true`, the runtime reserves through the selected relay set, not only the raw `relay_peers` list. Selected candidates may come from operator config, cache, rendezvous, or public fallback. When reservation is disabled, selected candidates are still seeded/dialed so the node can connect to known relay infrastructure without requesting reservations.

Failed peers are recorded in the peer cache through the existing outgoing-connection failure path. The `replace_failed_relays` flag is now part of policy and is reserved for the next runtime-health pass where failed reservations will trigger replacement from the remaining candidate pool.

## Observability

Snapshots and Prometheus-style metrics expose:

- selected relays
- total candidate count
- configured/cached/rendezvous/public candidate counts
- ignored candidate count
- discovery failures/warnings
- replacement counter

These fields let the GUI explain why a lite node has no relay, only one relay, or multiple relay fallback options.

## Public fallback relay candidates

`discovery.public_bootstrap.relay_peers` can add public relay/mediator candidates when public fallback policy allows it. These candidates are tracked separately from configured, cached, and rendezvous candidates so operators can see when the node depended on public infrastructure.

Public fallback relays are selected after operator-owned/cached/rendezvous candidates. The normal app default enables public fallback policy, but this shared library does not ship project-operated relay peers; app distributions can add public relay DNSADDR entries or use private `relay_peers`. See `docs/spec/PUBLIC_FALLBACK.md`.
