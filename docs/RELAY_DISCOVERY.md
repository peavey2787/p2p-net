# Relay discovery and selection

Phase 7 adds a first-class relay discovery policy so lite and mobile-lite nodes do not need every relay hard-coded forever.

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

All candidates must be identity-bound `/p2p/<PeerId>` multiaddrs with a reachable transport. Already-relayed `/p2p-circuit` addresses are not used as relay reservation targets.

## Selection rules

- Candidates are deduplicated by relay peer ID.
- Configured relays are preferred when `prefer_configured_relays = true`.
- Selection stops at `max_reservations`.
- A warning pulse is emitted when fewer than `min_reservations` candidates are available.
- Setting `enabled = false` preserves legacy behavior: only configured `relay_peers` are considered.

## Runtime behavior

Startup now reserves through the selected relay set instead of only raw `relay_peers`. Automatic cached/rendezvous relay discovery is enabled for lite/mobile-lite roles; full/infrastructure nodes keep legacy configured-relay behavior unless `relay_peers` are explicitly provided.

When `reserve_configured_relays = true`, selected candidates are converted to `/p2p-circuit` reservation listen addresses. When reservation is disabled, selected candidates are still seeded/dialed so the node can connect to known relay infrastructure without requesting reservations.

Failed peers are recorded in the peer cache through the existing outgoing-connection failure path. The `replace_failed_relays` flag is now part of policy and is reserved for the next runtime-health pass where failed reservations will trigger replacement from the remaining candidate pool.

## Observability

Snapshots and Prometheus-style metrics expose:

- selected relays
- total candidate count
- configured/cached/rendezvous candidate counts
- ignored candidate count
- discovery failures/warnings
- replacement counter

These fields let the GUI explain why a lite node has no relay, only one relay, or multiple relay fallback options.
