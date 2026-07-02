# Operator guides

These guides explain the normal consumer public-network default and the advanced/operator path for private infrastructure.

- `CONSUMER_DEFAULT.md` walks through the normal no-config user flow: public fallback joins discovery, app peers may auto-connect at the network layer, and contact trust still requires QR/join-code/invite verification.
- `PUBLIC_FALLBACK_TRADEOFFS.md` explains the normal public-fallback default and its privacy, abuse, performance, and dependency tradeoffs.
- `PRIVATE_INFRASTRUCTURE_FIRST.md` shows the advanced/operator config that disables public fallback and prefers owned bootstrap, rendezvous, mediator, relay, peer-cache, and DHT provider discovery paths.
- `PRODUCTION_FLEET.md` describes how to run a small owned bootstrap/rendezvous/mediator/relay fleet for production applications.

Example JSON configs live under `examples/` and validate as `NodeConfig` inputs. Start normal app documentation from `examples/consumer-default.config.json`; use `examples/private-infrastructure-first.config.json` only for Advanced/operator private mode.
