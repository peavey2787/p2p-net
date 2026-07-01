# Implementation docs

Implementation-facing guides for maintainers working on the Rust core,
transports, runtime policies, bindings, discovery, and observability.

- `API_IMPLEMENTATION.md` — six-primitives command routing and message delivery.
- `BEHAVIOUR_POLICY.md` — profile-driven libp2p behaviour construction.
- `BINDINGS.md` — binding-safe facade for desktop/mobile/WebView shells.
- `DCUTR_POLICY.md` — DCUtR upgrade policy, retry budget, and fallback counters.
- `DISCOVERY_NAMESPACE_IMPLEMENTATION.md` — runtime namespace derivation and config plumbing.
- `DHT_PROVIDER_DISCOVERY_IMPLEMENTATION.md` — provider-record startup wiring, event handling, and metrics.
- `DNSADDR_DOH.md` — bounded `/dnsaddr` DNS-over-HTTPS resolver policy.
- `EVENT_HANDLING.md` — single-responsibility swarm event handler split.
- `MEDIATOR.md` — first-class mediator role mapped to relay-server capability.
- `PLATFORM_RUNTIME.md` — platform runtime and storage adapter boundaries.
- `RELAY_DISCOVERY.md` — relay candidate selection and reservation policy.
- `PUBLIC_FALLBACK_IMPLEMENTATION.md` documents how the startup policy is wired.
