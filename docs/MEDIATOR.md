# Full, relay, mediator, lite, and mobile-lite roles

`p2p-net` keeps one shared node implementation. The runtime role is selected by the profile/capability resolver instead of by separate node implementations.

## Role definitions

- **Full node**: listens for inbound peers, participates directly in discovery/gossip, and can accept direct connections when the environment allows it.
- **Relay node**: a full/infrastructure node with Circuit Relay v2 server capability enabled for generic relayed traffic.
- **Mediator node**: a relay-capable full node that is intentionally configured to help lite/mobile peers establish relayed connectivity and then attempt DCUtR hole punching. In Phase 4 this maps onto Circuit Relay v2 server behaviour plus explicit mediator policy and metrics.
- **Lite node**: outbound-first node for NAT/CGNAT environments. It keeps relay client and DCUtR capability enabled, requests configured relay reservations when available, and avoids infrastructure server roles.
- **Mobile lite node**: lite node with stricter mobile/tablet assumptions: no public listener requirement, no relay/rendezvous server role, and background/battery-sensitive defaults.

## Mediator configuration

Minimal DCUtR mediator profile:

```json
{
  "profile": "mediator"
}
```

Equivalent explicit mediator config:

```json
{
  "mediator": {
    "enabled": true,
    "advertise_for_dcutr": true,
    "allow_reservations": true,
    "allow_circuits": true,
    "require_authenticated_peers": false,
    "max_mediated_peers": 64,
    "max_reservations": 64,
    "max_circuits": 128
  }
}
```

`mediator.enabled = true` intentionally enables the underlying relay server capability through the central capability resolver. This keeps the libp2p mapping DRY while making the operator intent clear.

## Authentication policy

Phase 4 maps `mediator.require_authenticated_peers = true` onto the existing relay allow-list policy. If this flag is enabled, `relay.allow_peers` must contain at least one trusted peer ID.

Example:

```json
{
  "mediator": {
    "enabled": true,
    "require_authenticated_peers": true
  },
  "relay": {
    "access": "allow_list",
    "allow_peers": ["12D3KooW..."]
  }
}
```

Current ACL enforcement is connection-level. A peer denied from mediator/relay use is denied from connecting to that node at all.

## Metrics and snapshot fields

Phase 4 exposes mediator-specific fields alongside the existing relay counters:

- `mediator_enabled`
- `mediator_advertise_for_dcutr`
- `mediator_require_authenticated_peers`
- `mediator_active_reservations`
- `mediator_active_circuits`
- `mediator_dcutr_attempts_observed`
- `mediator_denied_reservations`
- `mediator_denied_circuits`
- `mediator_abuse_rate_limit_events`

Prometheus-style metrics include:

- `p2p_mediator_enabled`
- `p2p_mediator_active_reservations`
- `p2p_mediator_active_circuits`
- `p2p_mediator_dcutr_attempts_observed`
- `p2p_mediator_denied_reservations`
- `p2p_mediator_denied_circuits`
- `p2p_mediator_abuse_rate_limit_events`

## Current limits

Phase 4 makes mediator intent explicit and maps it to Circuit Relay v2. It does not yet implement automatic relay/mediator discovery. That is Phase 7.

Lite nodes can already prefer known mediator-capable relays by putting them in `relay_peers` and leaving `reserve_configured_relays = true`.
