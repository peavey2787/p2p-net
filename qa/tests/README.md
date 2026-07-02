# Domain-grouped QA tests

Integration, system, invariant, hygiene, and doc-consistency tests live under domain folders here and are registered explicitly in the root `Cargo.toml`.

The domain folders keep `qa/tests/` from becoming a flat dumping ground:

- `api/` — public primitives and binding facade checks.
- `config/` — config defaults, environment detection, dependency policy, and platform build assumptions.
- `discovery/` — discovery namespaces, DHT provider records, peer book behavior, connection planning, rendezvous, and resurrection tests.
- `hygiene/` — repository layout, registration, SRP, and documentation drift checks.
- `observability/` — snapshot JSON and Prometheus metric checks.
- `operator/` — operator docs and example config validation.
- `relay/` — relay client/server, relay discovery, mediator, and DCUtR policy checks.
- `runtime/` — stack behavior, capability resolution, platform runtime, swarm smoke, and event-responsibility checks.
- `security/` — hostile-network, message-security, pulse-validation, and multi-node hostile checks.

When adding a new integration test, place it in the most specific domain folder and add a matching `[[test]]` target in the root `Cargo.toml`.
