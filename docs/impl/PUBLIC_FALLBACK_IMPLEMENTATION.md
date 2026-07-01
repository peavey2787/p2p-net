# Public fallback implementation

The public fallback implementation is intentionally small and policy-driven.

- `crates/connectivity/public_fallback.rs` owns `PublicBootstrapConfig`, `PublicFallbackMode`, and decision helpers.
- `crates/connectivity/discovery.rs` embeds the policy under `DiscoveryConfig.public_bootstrap`.
- `crates/node/types.rs` validates and parses public bootstrap/relay multiaddrs with the same identity-bound `/p2p/<PeerId>` requirements used by operator infrastructure.
- `crates/node/mod.rs` evaluates startup decisions, adds public bootstrap seeds to the startup dial plan when allowed, and adds public relay candidates to relay selection when allowed.
- `crates/connectivity/relay_discovery.rs` tracks public fallback relay candidates as their own source instead of merging them into configured relays.

The public fallback path does not change the six application primitives. Applications still call `get_peers()` and `connect_peer(...)`; discovery policy only changes which peers can be discovered or selected internally.

## Why there are no hidden defaults

Public bootstrap and relay infrastructure can have availability, privacy, abuse, policy, and performance tradeoffs. The runtime therefore ships with `mode = "disabled"` and empty public peer lists. Operators can still add known public libp2p/IPFS bootstrap or relay addresses in config for recovery scenarios.
