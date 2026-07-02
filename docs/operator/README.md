# Operator guides

These guides explain the normal public-fallback app default and the advanced/operator path for private infrastructure.

- `PRIVATE_INFRASTRUCTURE_FIRST.md` shows the advanced/operator config that disables public fallback and prefers owned bootstrap, rendezvous, mediator, relay, peer-cache, and DHT provider discovery paths.
- `PUBLIC_FALLBACK_TRADEOFFS.md` explains the normal public-fallback default and its privacy, abuse, performance, and dependency tradeoffs.
- `PRODUCTION_FLEET.md` describes how to run a small owned bootstrap/rendezvous/mediator/relay fleet for production applications.

Example JSON configs live under `examples/` and validate as `NodeConfig` inputs.
