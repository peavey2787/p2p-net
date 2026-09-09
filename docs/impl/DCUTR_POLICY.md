# DCUtR policy and fallback

Direct Connection Upgrade through Relay (DCUtR) is an explicit node policy instead of an implicit side effect of having the libp2p behaviour installed.

DCUtR is used when two peers first have a relayed path and then try to upgrade to a direct connection through hole punching. It still needs a relay/mediator path for discovery and coordination. The safe production strategy is:

```text
direct when possible
relay when necessary
upgrade when possible
keep relay fallback when upgrade fails
```

## Configuration

`NodeConfig.dcutr` controls the policy:

```json
{
  "dcutr": {
    "enabled": true,
    "attempt_after_relay_connection": true,
    "keep_relay_fallback": true,
    "retry_interval_secs": 60,
    "max_attempts_per_peer": 3
  }
}
```

Fields:

- `enabled`: installs the DCUtR behaviour when the resolved node capability set also allows relay-client fallback.
- `attempt_after_relay_connection`: treats intended relayed destination peers as eligible for a direct upgrade. rust-libp2p owns the protocol-level hole punch; p2p-net gates the peer and exposes counters.
- `keep_relay_fallback`: keeps the relay circuit available when an upgrade fails or cannot be attempted. This is required when upgrade-after-relay is enabled.
- `retry_interval_secs`: minimum spacing between policy-approved DCUtR handler activations for the same peer. It does not pace every wire-level attempt inside that handler.
- `max_attempts_per_peer`: caps those handler activations while the peer remains in the bounded policy bookkeeping, not the total number of transport dials.

## Profile resolution

The central capability resolver maps `DcutrPolicy` into `ResolvedNodeConfig`:

- lite and mobile-lite nodes keep relay-client and DCUtR enabled by default.
- disabling `dcutr.enabled` removes the DCUtR capability from behaviour construction.
- DCUtR cannot be enabled without relay-client fallback.
- unsafe upgrade-without-fallback combinations are rejected before startup.

## Observability

Snapshots and Prometheus-style metrics expose:

- `dcutr_enabled`
- `dcutr_attempts`
- `dcutr_successes`
- `dcutr_failures`
- `dcutr_relay_fallbacks`
- `dcutr_upgrade_eligible_connections`
- `dcutr_retry_suppressed`
- `dcutr_retry_interval_secs`
- `dcutr_max_attempts_per_peer`

The dashboard also shows DCUtR enabled/attempt/success/failure/fallback/suppression counters.

## Scope

This crate does not replace rust-libp2p's DCUtR protocol implementation. It wraps the behaviour with product-level policy before a relayed connection gets a DCUtR handler: only intended app/manual/cache-discovered destination peers are eligible, `retry_interval_secs` is enforced as a per-peer cooldown, and `max_attempts_per_peer` caps repeated relayed-upgrade attempts. Public relay servers are not namespace-filtered; the namespace/app-peer gate applies to the relayed destination peer behind the relay.

The cooldown above applies to **handler activation**, not every protocol retry.
The installed `libp2p-dcutr` 0.14.1 can initiate up to three direct upgrade rounds
inside one handler, retrying after a failed dial without waiting for
`retry_interval_secs`. Each round can dial multiple candidate addresses. Thus
the default values do not mean exactly three socket attempts, each 60 seconds
apart. Activating a handler also does not schedule a later retry on an existing
idle circuit when its cooldown expires.

`dcutr_attempts` counts policy-approved attempts started from eligible relayed connections. `dcutr_successes` and `dcutr_failures` count libp2p DCUtR result events, so operators can distinguish attempts started from result events completed.

## Isolating a live failure

`examples/live_dcutr_process_probe.rs` runs production nodes with a hard
60-second direct-upgrade acceptance window. Its optional `--tcp-control`
mode instead runs stock libp2p TCP/Noise/Yamux, Identify, relay and DCUtR,
without the application planner or DHT. It requires two roles, a fresh shared
session directory, the same public relay multiaddr, each node's local TCP port,
and each node's independently verified public IPv4 address. The shared files
carry identities/readiness; direct candidates travel through DCUtR over the
public relay. This diagnostic uses no LAN dial path and is **not** a substitute
for production-node acceptance. A relay-only connection fails both probes.
