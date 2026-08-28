# DHT Provider Discovery Specification

## Purpose

DHT provider discovery is the public decentralized discovery layer for application namespaces. It lets nodes announce and search for network-scoped namespace keys through Kademlia provider records, both with and without operator rendezvous infrastructure.

The public application data-plane API remains the six primitives:

- `connect_peer`
- `disconnect_peer`
- `send_message`
- `broadcast`
- `subscribe`
- `get_peers`

DHT provider discovery is internal plumbing behind those primitives.

## Priority order

The normal planner combines these sources without treating routing peers as application peers:

1. same-LAN compatibility-scoped discovery when enabled
2. operator bootstrap/rendezvous/relay infrastructure when configured
3. healthy peer cache
4. public bootstrap when fallback policy allows it
5. network-scoped DHT provider lookups
6. direct address recovery, dynamic relay fallback, and DCUtR upgrade attempts

Public fallback is enabled by default in fallback-only mode for normal app startup. DHT provider discovery may be enabled by default, but it only becomes useful after the node has joined a Kademlia routing layer through configured, cached, or fallback bootstrap peers. Private deployments can disable public fallback and use owned bootstrap/rendezvous infrastructure instead.

## Namespace keys

DHT provider keys use the same derived discovery namespaces as rendezvous discovery:

```text
p2p-net/<network_id>/<app_id>/<tag_hash>
```

Raw contact tags must not be published by default. The namespace model hashes app tags unless the operator explicitly enables readable unsafe tags. If no tags are configured and the built-in rendezvous namespace is unchanged, p2p-net derives a reserved hashed default tag from the exact `network_id` and app ID. Fresh default networks therefore no longer collide on one generic `p2p-net` provider key.

## Announcement

When `discovery.dht.enabled` and `discovery.dht.announce` are true, startup calls Kademlia `start_providing(namespace_key)` for each derived namespace, bounded by `max_namespaces_per_refresh`. Startup refresh retries use 5/15/30/60-second backoff, recovery from zero connected peers may accelerate a pending refresh with a 5-second minimum gap, and steady-state refreshes use `refresh_interval_secs`. Ordinary additional connections do not restart the refresh timer. Already-announced namespaces are not blindly re-submitted on every application heartbeat.
`periodic_bootstrap_interval_secs` controls libp2p Kademlia's separate routing-table bootstrap timer (`null` disables it), while `query_parallelism` bounds how many peers an iterative query waits on concurrently. `provider_key_replicas` selects one to three deterministic provider keys per namespace. Replica zero is common to every supported setting, while the production/full-node default remains three replicas. These controls are independent of namespace refresh timing.

## Discovery

When `discovery.dht.enabled` and `discovery.dht.discover` are true, startup calls Kademlia `get_providers(namespace_key)` for each derived namespace. The normal default keeps `discover_with_rendezvous_peers = true`, so DHT discovery continues alongside optional rendezvous infrastructure rather than becoming unavailable merely because a rendezvous peer is configured. Operators may disable that setting when they intentionally want rendezvous-only discovery.

Repeated runtime refreshes are throttled by:

```json
{
  "refresh_interval_secs": 300,
  "periodic_bootstrap_interval_secs": 300,
  "query_parallelism": 3,
  "provider_key_replicas": 3
}
```

Startup discovery still runs immediately; the interval controls repeated announce/query work after startup and public-IP probe refreshes.

## Runtime results

Provider lookup results update internal DHT provider state, observability counters, and the peer book used by `get_peers()`. Kademlia routing-table peers are not persisted as application peers merely because they helped route a query; only peers returned for the exact application namespace become application discovery candidates.

For each discovered provider, the runtime also tries to recover dialable addresses from Kademlia routing/provider information. A supplemental identity-signed `PeerRecord` is published under a namespace+peer key when the local node has a public direct or confirmed relay route; readers verify that the signed record identity exactly matches the provider peer before accepting any address. Relay reservation addresses are target-bound as `/p2p-circuit/p2p/<target>` before use. Public DHTs may apply their own record-storage policy, so signed records supplement rather than replace normal provider-address recovery.

When a new public or relay external address is confirmed, provider/address publication is refreshed immediately (subject to in-flight bounds), which prevents a startup provider record with no useful route from remaining stale for the full steady-state interval. Auto-dial retries are bounded and retain relay fallback while DCUtR attempts a direct upgrade.
