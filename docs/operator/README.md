# Operator guides

These guides explain the normal consumer public-network default and the advanced/operator path for private infrastructure.

- `CONSUMER_DEFAULT.md` walks through the normal no-config user flow: public fallback joins discovery, app peers may auto-connect at the network layer, and contact trust still requires QR/join-code/invite verification.
- `PUBLIC_FALLBACK_TRADEOFFS.md` explains the normal public-fallback default and its privacy, abuse, performance, and dependency tradeoffs.
- `PRIVATE_INFRASTRUCTURE_FIRST.md` shows the advanced/operator config that disables public fallback and prefers owned bootstrap, rendezvous, mediator, relay, peer-cache, and DHT provider discovery paths.
- `CHOOSING_A_ROLE.md` maps plain goals ("help browsers connect", "save battery") to one-word `profile` values and lists the settings power users can tune.
- `BROWSER_WEBRTC.md` covers browser-to-browser WebRTC as its own capability: automatic mode, WebRTC without TURN, STUN, automatic TURN fallback, TURN-only privacy mode, and how `browser_webrtc` and `turn.client` combine.
- `TURN.md` covers the built-in TURN server (`profile: "turn"`), the libp2p TURN credential broker, and Cloudflare Realtime TURN.
- `PRODUCTION_FLEET.md` describes how to run a small owned bootstrap/rendezvous/mediator/relay fleet for production applications.

Example JSON configs live under `examples/` and validate as `NodeConfig` inputs. TURN examples are `examples/turn-node.config.json` (one line), `examples/turn-node-advanced.config.json`, `examples/turn-cloudflare-broker.config.json`, and `examples/browser-turn-client.config.json`. Browser WebRTC examples are `examples/browser-webrtc-direct.config.json` (no TURN) and `examples/browser-turn-only-privacy.config.json` (relay-only ICE). Start normal app documentation from `examples/consumer-default.config.json`; use `examples/private-infrastructure-first.config.json` only for Advanced/operator private mode.
