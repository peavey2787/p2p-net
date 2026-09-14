# DHT Provider Discovery Implementation

## Modules

- `crates/connectivity/dht.rs` owns DHT namespace provider configuration, startup planning, provider-query tracking, and Kademlia event handling.
- `crates/connectivity/dht/keys.rs` owns deterministic provider-key derivation and public-bootstrap anchor key construction.
- `crates/node/events/kademlia.rs` updates node snapshots and operator pulses from Kademlia events.
- `crates/node/runtime/periodic.rs` starts DHT namespace announcement/discovery after a bounded relay/bootstrap head start.

## Startup behavior

Startup derives rendezvous/discovery namespaces from `DiscoveryConfig::rendezvous_namespaces(network_id)`. That preserves one namespace source for rendezvous and DHT provider discovery.

For every derived namespace up to `max_namespaces_per_refresh`:

1. `start_providing(namespace_key)` announces the local node as a provider when announcement is enabled.
2. `get_providers(namespace_key)` queries for other providers when discovery is enabled and the DHT policy says discovery should run.

The first announce/query starts after five seconds, giving relay reservation and bootstrap seed connections time to settle before iterative DHT queries fan out. Runtime retries then run 5, 10, 15, 30, and 60 seconds apart before using `discovery.dht.refresh_interval_secs`; recovery from zero connected peers or a newly learned external address may accelerate the next refresh, subject to the same five-second minimum gap. Ordinary additional connections do not reschedule discovery, preventing a query -> connection -> refresh feedback loop. Already-announced namespaces are not re-submitted every heartbeat.
The libp2p Kademlia behaviour separately receives `periodic_bootstrap_interval_secs` and `query_parallelism`. Seed insertion does not start an automatic full-bucket crawl, and startup does not explicitly duplicate that crawl: provider announce/lookups route directly from the configured seeds, while the configured periodic bootstrap maintains the routing table after startup settles. The swarm uses a replication factor of five per query instead of the public DHT's twenty-peer default. With three independent provider keys this retains fifteen public provider copies while allowing fresh-node announce/lookups to finish and retry within the 60-second connection budget. These settings change startup/query fan-out, not the node's role: full nodes remain Kademlia servers and continue serving inbound DHT requests. Provider-key derivation is cached in `DhtProviderState`; the anchor-key search also reuses a pre-hashed SHA-256 prefix rather than re-hashing invariant namespace/anchor material on every attempt. `provider_key_replicas` remains configurable and defaults to three: one stable key plus overlapping current/previous rolling keys. This redundancy prevents a single incomplete public-DHT traversal from hiding an application peer; operators can select one or two when lower DHT traffic matters more than startup discovery latency.

## Event behavior

`MeshEvent::Kademlia` is handled by `node/events/kademlia.rs` instead of falling through to generic discovery dispatch.

Handled results include:

- provider announce success/failure
- provider lookup results
- provider lookup completion/failure
- peer-ID auto-dials that allow Kademlia's routing/query state to supply
  addresses before they have been copied into the application peer book
- failed asynchronous provider dials become eligible for the next periodic
  provider result instead of being suppressed for the rest of the process
- routing updates and routable-peer address learning for already discovered provider peers

Kademlia routing-table peers are not written to the peer cache merely because they helped route a query. Address-learning events are promoted to the peer book/cache only for peers already discovered as providers of the application namespace.

## Observability

Snapshots and metrics expose:

- DHT provider enablement
- announce attempts/failures
- provider queries/failures
- provider records found
- discovered provider peer count

## Boundary

Provider results are recorded in `DhtProviderState` and mirrored into the peer book so `get_peers()` can return DHT-provider-discovered peers through the same normalized view used for connected, cached, rendezvous, relay, and configured peers.
