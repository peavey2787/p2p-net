# Public fallback implementation

The public fallback implementation is intentionally small and policy-driven. Normal app mode enables fallback-only public bootstrap by default; private-infrastructure-first operation disables it explicitly.

- `crates/connectivity/public_fallback.rs` owns `PublicBootstrapConfig`, `PublicFallbackMode`, and decision helpers.
- `crates/connectivity/discovery.rs` embeds the policy under `DiscoveryConfig.public_bootstrap`.
- `crates/node/config_validation.rs` validates public bootstrap/relay multiaddrs with the same identity-bound `/p2p/<PeerId>` requirements used by operator infrastructure; `crates/node/config.rs` exposes the parsed startup address accessors.
- `crates/node/startup.rs` evaluates startup decisions, adds public bootstrap seeds to the startup dial plan when allowed, and adds public relay candidates to relay selection when allowed.
- `crates/connectivity/relay_discovery.rs` tracks public fallback relay candidates as their own source instead of merging them into configured relays.

The public fallback path does not change the six application primitives. Applications still call `get_peers()` and `connect_peer(...)`; discovery policy only changes which peers can be discovered or selected internally.

## Default behavior

The runtime ships with `mode = "fallback_only"` and a built-in public bootstrap seed list so regular users do not have to configure bootstrap peers manually. Public fallback DNS resolution is best-effort: bad multiaddr syntax still fails validation, but temporary public DNS/bootstrap outages do not stop local startup.

The shared library does not ship a project-operated public relay fleet. App distributions that need guaranteed NAT-to-NAT default connectivity should add real public relay/mediator DNSADDR entries to `discovery.public_bootstrap.relay_peers`, or operate private relays and put them in `relay_peers`.
