# DCUtR policy and fallback

Phase 8 makes Direct Connection Upgrade through Relay (DCUtR) an explicit node policy instead of an implicit side effect of having the libp2p behaviour installed.

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
- `attempt_after_relay_connection`: treats relayed connections as eligible for a direct upgrade. rust-libp2p owns the protocol-level hole punch; p2p-net records the policy and exposes counters.
- `keep_relay_fallback`: keeps the relay circuit available when an upgrade fails or cannot be attempted. This is required when upgrade-after-relay is enabled.
- `retry_interval_secs`: minimum future retry spacing for repeated attempts to the same peer. Phase 8 exposes and validates the policy; deeper timer-driven retries can build on it without changing config shape.
- `max_attempts_per_peer`: caps repeated upgrade attempts before relying on relay fallback.

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

## What this phase does not do

Phase 8 does not replace rust-libp2p's DCUtR implementation. It adds the product-level policy, validation, resolved capability wiring, safe fallback accounting, and operator-visible counters. Live timer-driven retry scheduling and relay replacement can now be layered on top of the policy without spreading DCUtR decisions through unrelated modules.
