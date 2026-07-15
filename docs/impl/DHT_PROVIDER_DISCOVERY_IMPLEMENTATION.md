# DHT Provider Discovery Implementation

## Modules

- `crates/connectivity/dht.rs` owns DHT namespace provider configuration, startup planning, provider-query tracking, and Kademlia event handling.
- `crates/node/events/kademlia.rs` updates node snapshots and operator pulses from Kademlia events.
- `crates/node/mod.rs` starts DHT namespace announcement/discovery after rendezvous startup planning.

## Startup behavior

Startup derives rendezvous/discovery namespaces from `DiscoveryConfig::rendezvous_namespaces(network_id)`. That preserves one namespace source for rendezvous and DHT provider discovery.

For every derived namespace up to `max_namespaces_per_refresh`:

1. `start_providing(namespace_key)` announces the local node as a provider when announcement is enabled.
2. `get_providers(namespace_key)` queries for other providers when discovery is enabled and the DHT policy says discovery should run.

Startup performs the first announce/query immediately. Runtime refreshes are throttled by `discovery.dht.refresh_interval_secs` and already-announced namespaces are not re-submitted every heartbeat.

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
