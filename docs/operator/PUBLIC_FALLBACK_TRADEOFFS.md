# Public fallback tradeoffs

Public fallback is the normal app default. It is meant to give regular users a no-config startup path: run the app, use the built-in public bootstrap list plus any app-distribution public rendezvous/relay entries, discover peers, and then let peer cache, DHT provider discovery, rendezvous, relay policy, and network-layer auto-connect maintain connectivity.

Power users and operators can still disable public fallback or replace it with private infrastructure. See `CONSUMER_DEFAULT.md` for the normal app walkthrough and `PRIVATE_INFRASTRUCTURE_FIRST.md` for the Advanced/operator override.

## Policy modes

- `fallback_only`: default normal app mode. Use public candidates only when owned startup, rendezvous, or relay candidates are empty.
- `disabled`: advanced private-infrastructure-only mode. Never use public bootstrap, public rendezvous, or public relay candidates.
- `always`: include public candidates after owned candidates on every startup.

For consumer app defaults, use `fallback_only`. For private deployments, use `disabled` with owned bootstrap/rendezvous/relay peers.


## Example config

The consumer-facing example is tracked at `examples/consumer-default.config.json`. The expanded public-fallback operator example is tracked at `examples/public-fallback.config.json`. Both keep `fallback_only` enabled, leave manual bootstrap overrides optional, enable network-layer auto-connect for discovered peers, and keep public rendezvous/relay entries empty unless the application operates or contracts real infrastructure.


## Bootstrap, rendezvous, relay, and trust are separate

Public bootstrap gives a node a first route into the wider libp2p/Kademlia layer. The default runtime then uses network-scoped DHT provider discovery plus signed peer-address recovery to find application peers, discovers relay-hop-capable public DHT peers when NAT traversal needs a circuit, and attempts DCUtR while retaining relay fallback. Same-LAN nodes use the faster UDP discovery path.

`auto_connect_discovered_peers` only creates bounded network-layer dial attempts. It must not add trusted contacts. Contact trust still belongs to the application layer through QR codes, join codes, invite acceptance, safety-number verification, or an equivalent explicit user action.

## Privacy tradeoffs

Public bootstrap, rendezvous, or relay operators can observe connection attempts, source IPs, timing, and relay usage. They should not receive raw application contact tags because discovery namespaces are hashed by default, but metadata can still reveal activity patterns. Use private infrastructure when contact-graph privacy matters.

## Abuse tradeoffs

Public relays are shared infrastructure. They may rate-limit, block, log, or reject traffic according to their own policy. Your application should treat them as best-effort recovery paths, not as guaranteed service capacity. If your app can generate high traffic, run owned relays and set explicit relay limits and ACLs.

## Performance tradeoffs

Public fallback usually adds more latency, more variable availability, and less predictable relay bandwidth than a fleet you operate near your users. Relayed traffic is also more expensive than direct traffic. Keep DCUtR enabled with relay fallback retained so the node can attempt direct upgrade when a relay path exists.

## Dependency tradeoffs

A public fallback list is a dependency on external operators. Those peers can disappear, change policy, or become overloaded without notice. Keep the fallback mode visible in settings, monitor `public_fallback_used` plus the category-specific `public_bootstrap_used`, `public_rendezvous_used`, and `public_relay_used` fields, and let advanced users disable public fallback or replace it with explicit bootstrap peers.

## Safer public-fallback checklist

- Keep `fallback_only` as the normal user default.
- Put manual bootstrap/rendezvous/relay peers under Advanced settings.
- Keep `auto_connect_discovered_peers` on for normal users, but never auto-trust those peers as contacts.
- Offer a clear private-infrastructure-only toggle that sets public fallback to `disabled`.
- Keep hashed discovery namespaces enabled.
- Avoid readable discovery tags outside local debugging.
- Watch snapshot and metrics fields for public fallback participation.
- If your deployment needs guaranteed relay capacity or service-level control, operate or contract rendezvous/relay peers and add them to the public-fallback lists; otherwise the default decentralized relay path remains best-effort external infrastructure.
