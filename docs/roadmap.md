# p2p-net roadmap

Goal: keep one shared Rust P2P core while letting the same node run as a full node, lite node, relay/DCUtR mediator, rendezvous server, bootstrap node, desktop node, or mobile/tablet node through profile-driven capabilities instead of separate implementations.

## Repository layout

```text
crates/
  p2p-net/
    src/
      bindings/
      common/
      connectivity/
      node/
      platform/
      protocol/
      stack/

docs/
  future-work/
  impl/
  project/
  spec/
  validation/
  roadmap.md

qa/
  ci/
  fuzz/
  tools/
  vectors/
  tests/

examples/
external/
  vendor/
```

## Completed milestones

1. Explicit node profiles and resolved capability view.
2. Environment detection for platform/reachability hints.
3. Central capability resolver.
4. First-class mediator profile and config.
5. Single-responsibility swarm event handlers.
6. Profile-driven behaviour construction.
7. Relay discovery and selection policy.
8. DCUtR policy and fallback counters.
9. Platform runtime and storage abstraction.
10. Binding-safe facade for desktop, Android, iOS/iPadOS, and restricted host shells.
11. Cleanup/hygiene pass for stale wording, duplicate test registration, and snapshot JSON drift.
12. Repository reorganization into `crates/`, `docs/`, `qa/`, `examples/`, and `external/`.

## Current architecture guardrails

- Runtime source code lives under `crates/p2p-net/src/`.
- Global/system/invariant tests live under `qa/tests/` and are explicitly registered in `Cargo.toml`.
- The canonical validation entry points are `qa/ci/run-full-validation.ps1` and `qa/ci/run-full-validation.sh`.
- Fuzzing targets live under `qa/fuzz/`.
- Internal QA utilities live under `qa/tools/`.
- Protocol fixtures and deterministic test vectors belong under `qa/vectors/`.
- Implementation docs live under `docs/impl/`.
- Core specifications live under `docs/spec/`.
- Validation docs live under `docs/validation/`.
- AI/project summaries and audits belong under `docs/project/`.
- Deferred ideas and long-term proposals belong under `docs/future-work/`.
- Third-party local patches live under `external/vendor/`.

## Remaining production work

These are deployment/product hardening items outside the core refactor:

- Host-language generated binding tests for Kotlin/Swift once a binding generator is chosen.
- Real multi-host soak tests across representative NAT, firewall, relay, and mobile-background conditions.
- External security review of relay abuse policy, DNS resolution policy, and identity-key lifecycle.
- Operational deployment runbooks for monitoring, alerting, relay capacity planning, and incident response.
- Protocol/test vectors for any wire format that becomes stable public API.
- App-shell UX for identity backup, restore, rotation, safety-number verification, and relay/mediator selection.

## Progress ledger

| Date | Milestone | Status | Notes |
|---|---:|---|---|
| 2026-07-01 | 1 | Done | Added explicit profiles, roles, behaviour-set view, resolved config, profile defaults, and tests. |
| 2026-07-01 | 2 | Done | Added environment reports, platform/reachability hints, and auto-profile resolution. |
| 2026-07-01 | 3 | Done | Centralized capability resolution and pre-start validation. |
| 2026-07-01 | 4 | Done | Added first-class mediator config/profile and relay-server mapping. |
| 2026-07-01 | 5 | Done | Split swarm event handling into focused responsibility modules. |
| 2026-07-01 | 6 | Done | Made behaviour construction consume resolved policy; Kademlia mode is profile-driven. |
| 2026-07-01 | 7 | Done | Added relay discovery/selection from configured, cached, and rendezvous candidates. |
| 2026-07-01 | 8 | Done | Added explicit DCUtR policy, retry/fallback counters, and observability. |
| 2026-07-01 | 9 | Done | Added platform runtime/storage traits and desktop/mobile/memory adapters. |
| 2026-07-01 | 10 | Done | Added binding-safe facade and CI matrix coverage for Linux/Windows/macOS. |
| 2026-07-01 | Cleanup | Done | Removed stale phase-specific tooling, cleaned transitional wording, and added hygiene guards. |
| 2026-07-01 | Reorg | Done | Moved code to `crates/`, validation to `qa/`, docs into categorized subfolders, and local patches to `external/vendor/`. |
