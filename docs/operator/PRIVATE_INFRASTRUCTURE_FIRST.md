# Private-infrastructure-first operation

The default production shape is to use infrastructure you operate or explicitly trust before using any public bootstrap or relay resource.

Recommended client startup order:

1. Dial pinned owned bootstrap peers from `bootstrap_peers` and `discovery.bootstrap_seed_peers`.
2. Dial owned rendezvous servers from `discovery.rendezvous_peers`.
3. Dial and reserve through owned mediator/relay peers from `relay_peers`.
4. Reuse healthy identity-bound peer-cache entries.
5. Announce and query hashed application namespaces through DHT provider records.
6. Keep public fallback disabled unless the deployment has an explicit recovery policy.

Use `examples/private-infrastructure-first.config.json` as the starting client config.

Important properties of that example:

- `discovery.public_bootstrap.mode` is `disabled`.
- `/dnsaddr` points at an operator-owned DoH endpoint instead of a hard-coded third-party dependency.
- `discovery.namespace.tags` are configured, while `privacy` remains `hashed` and `allow_readable_tags` remains `false`.
- `discovery.rendezvous.client_enabled` is true so clients register and discover through owned rendezvous peers.
- `discovery.dht` remains enabled so hashed namespace provider records can recover peers when rendezvous is unavailable.
- `relay_peers` contains owned mediator/relay addresses and `reserve_configured_relays` is true.
- `dcutr.keep_relay_fallback` is true so a relayed path remains usable when direct upgrade does not work.

Before using the example in production, replace every example address, domain, peer id, and key path with deployment-specific values.
