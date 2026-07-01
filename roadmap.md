# p2p-net refactor roadmap

Goal: keep one shared Rust P2P core while letting the same node run as a full node, lite node, relay/DCUtR mediator, rendezvous server, bootstrap node, desktop node, or mobile/tablet node through profile-driven capabilities instead of separate implementations.

The target architecture is documented with a `crates/` workspace layout. The current repository still has a single crate layout; migration into `crates/p2p-net` should be a dedicated step so profile changes, transport changes, and workspace movement do not get mixed together.

## Target crate layout

```text
crates/
  p2p-net/
    node/
      builder.rs
      runtime.rs
      handle.rs
      snapshot.rs
    config/
      raw.rs
      profile.rs
      resolved.rs
      validation.rs
    capabilities/
      detector.rs
      resolver.rs
      environment.rs
    roles/
      full.rs
      lite.rs
      relay.rs
      rendezvous.rs
      bootstrap.rs
    transport/
      builder.rs
      policy.rs
      addresses.rs
    relay/
      client.rs
      server.rs
      mediator.rs
      reservation.rs
      policy.rs
    dcutr/
      policy.rs
      events.rs
      metrics.rs
    discovery/
      bootstrap.rs
      peer_cache.rs
      rendezvous.rs
      relay_discovery.rs
    platform/
      traits.rs
      desktop.rs
      mobile.rs
      memory.rs
    protocol/
      pulse.rs
      reputation.rs
```

## Phase 1 — Explicit node profiles and resolved capability view

Status: implemented.

Add a user-facing profile layer without splitting the runtime yet.

Required work:

- Add `NodeProfile` with `auto`, `full`, `lite`, `relay`, `rendezvous`, `bootstrap`, and `mobile_lite`.
- Add `NodeRole` so `auto` can resolve into a concrete role.
- Add a profile-level `BehaviourSet` that describes intended capabilities without exposing libp2p internals.
- Add `ResolvedNodeConfig` as the single future input to transport and behaviour construction.
- Keep backward compatibility: old configs that omit `profile` must still load as `auto`.
- Apply conservative phase-1 profile defaults at node startup:
  - `relay` profile enables the relay server capability.
  - `lite` and `mobile_lite` disable relay server/rendezvous server infrastructure.
  - `rendezvous` profile enables rendezvous client and server.
  - `auto` preserves existing behaviour until environment detection exists.
- Add unit tests for profile defaults and resolved roles.

Exit criteria:

- `NodeConfig::default()` resolves to the current behaviour-compatible role.
- Existing JSON configs still load.
- Explicit profiles are serializable/deserializable.
- The runtime has one obvious place where profile defaults are applied.

## Phase 2 — Environment detection

Status: implemented.

Add a capability detector that reports platform and network reachability.

Required work:

- Add `EnvironmentReport` containing platform, reachability, NAT status, listen capability, CGNAT likelihood, battery sensitivity, and background restrictions.
- Add `EnvironmentConfig` hints for platform shells and tests.
- Add advisory methods for NAT observations, observed external addresses, and listen failures without coupling policy to libp2p event types.
- Keep detection advisory-only at first; do not make the node unstable by changing roles mid-run.
- Add tests for public desktop, private NAT desktop, CGNAT-like desktop, Android, iOS, unknown platform reports, explicit profile overrides, and runtime observation updates.

Exit criteria:

- `profile = auto` can be resolved using environment information, not only static config.
- The detector is separated from libp2p behaviour assembly.

## Phase 3 — Capability resolver

Status: implemented.

Centralize all role/profile decisions into one resolver.

Required work:

- Add `resolve_node_config(raw, environment) -> ResolvedNodeConfig`.
- Move all scattered decisions about relay, DCUtR, Kademlia, rendezvous, listening, and relay reservation into the resolver.
- Make the runtime consume `ResolvedNodeConfig` instead of raw config where possible.
- Validate impossible combinations before starting the swarm.

Exit criteria:

- The runtime does not independently decide what a lite/full/relay/mobile node means.
- New profiles can be added without editing unrelated modules.

## Phase 4 — First-class mediator role

Status: implemented.

Make DCUtR mediation explicit instead of only implying it through `relay.enabled`.

Required work:

- Add `MediatorConfig` with reservation, circuit, auth, advertisement, and capacity controls.
- Map mediator policy to Circuit Relay v2 server behaviour.
- Expose mediator metrics:
  - active reservations
  - active circuits
  - DCUtR attempts observed
  - denied reservations
  - denied circuits
  - abuse/rate-limit events
- Document the distinction between full node, relay node, mediator node, lite node, and mobile lite node.

Exit criteria:

- Operators can intentionally run a DCUtR mediator.
- Lite nodes can prefer known mediator-capable relays.

## Phase 5 — Single-responsibility event handling

Status: implemented.

Split the large swarm event handling path by responsibility.

Required work:

- Keep top-level event dispatch in `node/events` only.
- Move relay client events to relay client handling.
- Move relay server/mediator events to relay server handling.
- Move DCUtR events to DCUtR handling.
- Move rendezvous events to discovery/rendezvous handling.
- Move gossip validation events to protocol/gossip handling.
- Move connection cap/snapshot updates into connectivity/metrics handling.

Exit criteria:

- Each event module has one clear responsibility.
- Relay, DCUtR, rendezvous, and gossip logic can be tested independently.

## Phase 6 — Profile-driven behaviour construction

Status: implemented.

Make libp2p behaviour construction consume resolved capabilities.

Required work:

- Make `build_behaviour` accept `ResolvedNodeConfig` or a narrow behaviour policy.
- Stop hard-coding Kademlia server mode for every node.
- Disable infrastructure behaviours for lite/mobile profiles.
- Keep relay client and DCUtR enabled for lite/mobile profiles.
- Keep relay server enabled only for relay/mediator/full nodes that explicitly allow it.

Exit criteria:

- A lite node no longer behaves as an accidental Kademlia server.
- A mobile lite node avoids unnecessary infrastructure services.

Implementation notes:

- `build_swarm` now accepts `ResolvedNodeConfig` alongside the effective runtime config.
- `build_behaviour` consumes the resolved behaviour policy.
- Kademlia mode is selected from the resolved policy: infrastructure profiles use server mode; lite/mobile-lite use client mode.
- Relay server and rendezvous server behaviours are enabled only when both the resolved policy and effective config allow them.
- The transport plan reports `kademlia-client` or `kademlia-server` according to the resolved policy.

## Phase 7 — Relay discovery and selection

Status: planned.

Make lite mode usable without manually hard-coding every relay.

Required work:

- Add `RelayDiscoveryPolicy`.
- Load configured relays.
- Load cached healthy relays.
- Discover relays through rendezvous/bootstrap metadata.
- Maintain a minimum number of healthy reservations.
- Replace failed, slow, or abusive relays.
- Expose selected relays in node snapshot.

Exit criteria:

- Lite nodes can find and maintain relays automatically.
- Operators can still pin specific relays for controlled deployments.

## Phase 8 — DCUtR policy and fallback

Status: planned.

Turn DCUtR into a clear connection-upgrade policy.

Required work:

- Add `DcutrPolicy`.
- Attempt DCUtR after a relayed connection when policy allows it.
- Prefer direct connection when hole punching succeeds.
- Keep relay fallback when hole punching fails.
- Add retry/backoff and max-attempt controls.
- Add metrics for attempt, success, failure, and fallback.

Exit criteria:

- Connection strategy is explicit: direct when possible, relay when necessary, upgrade when possible.

## Phase 9 — Platform runtime abstraction

Status: planned.

Keep one P2P core while isolating desktop/mobile platform differences.

Required work:

- Add `NodeStorage` for identities, peer cache, and secrets.
- Add `PlatformRuntime` for platform kind, data directory, background limits, and listen capability.
- Add desktop implementation using standard filesystem and Tokio runtime.
- Add mobile implementation hooks for Android/iOS storage and lifecycle.

Exit criteria:

- The P2P core no longer assumes desktop filesystem/runtime everywhere.
- Android/iOS can use thin adapters instead of separate P2P implementations.

## Phase 10 — Cross-platform bindings and CI

Status: planned.

Expose the shared core to every app shell without duplicating networking logic.

Required work:

- Pick binding strategy: UniFFI, C ABI, flutter_rust_bridge, or other.
- Add Android/Kotlin binding smoke test.
- Add iOS/Swift binding smoke test.
- Add desktop integration smoke test.
- Add CI matrix for Windows, Linux, macOS, Android target, and iOS target where practical.

Exit criteria:

- Windows, Linux, macOS, Android, iOS, and tablets use the same core networking crate.
- Platform-specific code is limited to storage, lifecycle, permissions, UI, and packaging.

## Progress ledger

| Date | Phase | Status | Notes |
|---|---:|---|---|
| 2026-07-01 | 1 | Started | Added this roadmap and began explicit profile/resolved-capability implementation. |
| 2026-07-01 | 1 | Implemented | Added `NodeProfile`, `NodeRole`, `BehaviourSet`, `ResolvedNodeConfig`, profile defaults, example config update, and profile unit tests. Static edit only; cargo was unavailable in the sandbox for compile/test validation. |
| 2026-07-01 | 2 | Implemented | Added advisory `EnvironmentConfig`/`EnvironmentReport`, platform/reachability/NAT enums, auto-profile resolution with environment input, environment snapshot fields, example config hints, and environment detection tests. Also fixed a duplicated `NodeSnapshot` field found while preparing this phase. Static edit only; cargo was unavailable in the sandbox for compile/test validation. |
| 2026-07-01 | 3 | Implemented | Fixed clippy `derivable_impls` for `NodeProfile`, added the central `resolve_node_config(raw, environment)` capability resolver, added a runtime compatibility adapter for resolved capabilities, moved role decisions out of startup code, added pre-start validation for impossible capability combinations, and added `capability_resolver` unit tests. Static edit only; cargo was unavailable in the sandbox for compile/test validation. |
| 2026-07-01 | 4 | Implemented | Added first-class `MediatorConfig`, `mediator` profile/role, resolver mapping from mediator policy to Circuit Relay v2 server capability, mediator snapshot/JSON/Prometheus metrics, mediator role tests, example config fields, and `docs/MEDIATOR.md`. Static edit only; cargo was unavailable in the sandbox for compile/test validation. |
| 2026-07-01 | 5 | Implemented | Split swarm event handling into focused `node/events/` modules for connection lifecycle, relay client, relay server/mediator, DCUtR, rendezvous, and gossip validation. Kept `src/node/events.rs` as top-level dispatch only, added `docs/EVENT_HANDLING.md`, and registered an `event_responsibility` test. Static edit only; cargo was unavailable in the sandbox for compile/test validation. |
| 2026-07-01 | 6 | Implemented | Made swarm/behaviour construction consume `ResolvedNodeConfig`; Kademlia now uses server mode only for infrastructure profiles and client mode for lite/mobile-lite; relay/rendezvous server toggles now require both resolved policy and effective config; added `docs/BEHAVIOUR_POLICY.md` and registered a `behaviour_policy` test. Static edit only; cargo was unavailable in the sandbox for compile/test validation. |
