# Public fallback tradeoffs

Public fallback is the normal app default. It is meant to give regular users a no-config startup path: run the app, use the built-in public bootstrap list, discover peers, and then let peer cache, DHT provider discovery, rendezvous, and relay policy maintain connectivity.

Power users and operators can still disable public fallback or replace it with private infrastructure.

## Policy modes

- `fallback_only`: default normal app mode. Use public candidates only when owned startup candidates or owned relay candidates are empty.
- `disabled`: advanced private-infrastructure-only mode. Never use public bootstrap or public relay candidates.
- `always`: include public candidates after owned candidates on every startup.

For consumer app defaults, use `fallback_only`. For private deployments, use `disabled` with owned bootstrap/rendezvous/relay peers.

## Privacy tradeoffs

Public bootstrap or relay operators can observe connection attempts, source IPs, timing, and relay usage. They should not receive raw application contact tags because discovery namespaces are hashed by default, but metadata can still reveal activity patterns. Use private infrastructure when contact-graph privacy matters.

## Abuse tradeoffs

Public relays are shared infrastructure. They may rate-limit, block, log, or reject traffic according to their own policy. Your application should treat them as best-effort recovery paths, not as guaranteed service capacity. If your app can generate high traffic, run owned relays and set explicit relay limits and ACLs.

## Performance tradeoffs

Public fallback usually adds more latency, more variable availability, and less predictable relay bandwidth than a fleet you operate near your users. Relayed traffic is also more expensive than direct traffic. Keep DCUtR enabled with relay fallback retained so the node can attempt direct upgrade when a relay path exists.

## Dependency tradeoffs

A public fallback list is a dependency on external operators. Those peers can disappear, change policy, or become overloaded without notice. Keep the fallback mode visible in settings, monitor `public_fallback_used`, and let advanced users disable public fallback or replace it with explicit bootstrap peers.

## Safer public-fallback checklist

- Keep `fallback_only` as the normal user default.
- Put manual bootstrap/rendezvous/relay peers under Advanced settings.
- Offer a clear private-infrastructure-only toggle that sets public fallback to `disabled`.
- Keep hashed discovery namespaces enabled.
- Avoid readable discovery tags outside local debugging.
- Watch snapshot and metrics fields for public fallback participation.
- If your app needs NAT-to-NAT reliability for all users, operate or contract public relay/mediator peers and add them to `discovery.public_bootstrap.relay_peers`.
