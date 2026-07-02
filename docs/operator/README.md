# Operator guides

These guides explain how to run `p2p-net` with operator-owned discovery infrastructure first and public bootstrap or relay resources only when explicitly allowed.

- `PRIVATE_INFRASTRUCTURE_FIRST.md` shows a client/node config that prefers owned bootstrap, rendezvous, mediator, relay, peer-cache, and DHT provider discovery paths.
- `PUBLIC_FALLBACK_TRADEOFFS.md` shows the opt-in public fallback config and explains privacy, abuse, performance, and dependency tradeoffs.
- `PRODUCTION_FLEET.md` describes how to run a small owned bootstrap/rendezvous/mediator/relay fleet for production applications.

Example JSON configs live under `examples/` and validate as `NodeConfig` inputs.
