# Public fallback implementation

The public fallback implementation is policy-driven. Normal app mode enables fallback-only public bootstrap by default, exposes public rendezvous/relay candidate slots, and keeps network-layer auto-connect enabled for app-namespace-discovered peers. Private-infrastructure-first operation disables public fallback explicitly.

- `crates/connectivity/public_fallback.rs` owns `PublicBootstrapConfig`, `PublicFallbackMode`, decision helpers, and the `auto_connect_discovered_peers` policy flag.
- `crates/connectivity/discovery.rs` embeds the policy under `DiscoveryConfig.public_bootstrap`.
- `crates/node/config_validation.rs` validates public bootstrap/rendezvous/relay multiaddrs with the same identity-bound `/p2p/<PeerId>` requirements used by operator infrastructure; `crates/node/config.rs` exposes parsed startup address accessors.
- `crates/node/startup.rs` evaluates startup decisions, adds public bootstrap/rendezvous peers to startup discovery when allowed, and adds public relay candidates to relay selection when allowed.
- `crates/connectivity/relay_discovery.rs` tracks public fallback relay candidates as their own source instead of merging them into configured relays.

The public fallback path does not change the application data-plane primitives. Applications still call `get_peers()` and `connect_peer(...)`; discovery policy only changes which peers can be discovered or selected internally. Auto-connect is network-layer only and must not add trusted contacts. Runtime snapshots and Prometheus metrics expose auto-connect enabled state, dial attempts/failures, pending connection plans, and peers waiting for dialable addresses so known/discovered peer counts are not mistaken for established connections.

## Default behavior

The runtime ships with `mode = "fallback_only"`, a built-in public bootstrap seed list, rendezvous client enabled, and DHT provider discovery enabled alongside rendezvous. Regular users do not have to configure bootstrap peers manually.

Public fallback DNS resolution is best-effort: bad multiaddr syntax still fails validation, but temporary public DNS/bootstrap outages do not stop local startup.

The shared library does not ship a project-operated public rendezvous or relay fleet. App distributions that need guaranteed run-two-fresh-installs connectivity should add real public rendezvous and relay/mediator DNSADDR entries to `discovery.public_bootstrap.rendezvous_peers` and `discovery.public_bootstrap.relay_peers`, or operate private infrastructure and configure it through the advanced/operator fields.

## Consumer vs operator examples

- `examples/consumer-default.config.json` documents the normal user-facing behavior: public fallback on, manual bootstrap peers empty, public auto-connect on, rendezvous client on, DHT provider discovery on, and contact trust left to the application layer.
- `examples/public-fallback.config.json` is the expanded public-fallback example for operators and power users who want to see the full fallback shape.
- `examples/private-infrastructure-first.config.json` documents the Advanced/operator private mode: public fallback disabled, public auto-connect disabled, and owned bootstrap/rendezvous/relay peers supplied explicitly.

Public bootstrap alone is intentionally documented as insufficient for guaranteed NAT-to-NAT first launch. The implementation exposes public rendezvous and relay slots, but applications must fill those slots with real infrastructure before claiming reliable no-config connectivity.
