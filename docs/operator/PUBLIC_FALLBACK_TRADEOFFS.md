# Public fallback tradeoffs

Public bootstrap and relay fallback is intentionally opt-in. It can help a mesh recover when owned infrastructure and cached peers are unavailable, but it changes the trust and dependency model of the deployment.

Use `examples/public-fallback.config.json` only after choosing a clear fallback policy.

## Policy modes

- `disabled`: never use public bootstrap or public relay candidates.
- `fallback_only`: use public candidates only when owned startup candidates or owned relay candidates are empty.
- `always`: include public candidates after owned candidates on every startup.

For most production applications that allow public fallback at all, `fallback_only` is safer than `always`.

## Privacy tradeoffs

Public bootstrap or relay operators can observe connection attempts, source IPs, timing, and relay usage. They should not receive raw application contact tags because discovery namespaces are hashed by default, but metadata can still reveal activity patterns. Use owned infrastructure when contact-graph privacy matters.

## Abuse tradeoffs

Public relays are shared infrastructure. They may rate-limit, block, log, or reject traffic according to their own policy. Your application should treat them as best-effort recovery paths, not as guaranteed service capacity. If your app can generate high traffic, run owned relays and set explicit relay limits and ACLs.

## Performance tradeoffs

Public fallback usually adds more latency, more variable availability, and less predictable relay bandwidth than a fleet you operate near your users. Relayed traffic is also more expensive than direct traffic. Keep DCUtR enabled with relay fallback retained so the node can attempt direct upgrade when a relay path exists.

## Dependency tradeoffs

A public fallback list is a dependency on external operators. Those peers can disappear, change policy, or become overloaded without notice. Keep lists explicit in config, monitor `public_fallback_used`, and treat repeated public fallback use as an alert that owned infrastructure or peer-cache recovery is not working.

## Safer public-fallback checklist

- Use `fallback_only`, not `always`, unless the application deliberately wants public candidates on every startup.
- Keep owned bootstrap, rendezvous, mediator, and relay peers configured first.
- Keep hashed discovery namespaces enabled.
- Avoid readable discovery tags outside local debugging.
- Watch snapshot and metrics fields for public fallback participation.
- Document which public peer operators are trusted and why.
