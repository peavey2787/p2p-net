# Discovery namespace implementation

The namespace implementation lives in `crates/connectivity/namespace.rs` and is
wired through `DiscoveryConfig`.

Key types:

- `DiscoveryNamespaceConfig`
- `DiscoveryNamespacePrivacy`
- `DiscoveryNamespace`
- `build_discovery_namespace(...)`
- `discovery_tag_hash_hex(...)`

`DiscoveryConfig::rendezvous_namespaces(network_id)` returns the effective
namespace list used by rendezvous startup/refresh:

1. If app tags are configured, derive hashed app namespaces.
2. If no app tags are configured, use `discovery.rendezvous.namespace`.

This keeps the existing single-namespace operator flow working while allowing
applications to configure multiple hashed tags.

The runtime snapshot exposes:

- `discovery_namespace_mode`
- `discovery_namespaces`
- `discovery_namespace_count`

Prometheus-style metrics include:

```text
p2p_discovery_namespace_count
```


Rendezvous discovery requests are kept to one in-flight namespace per rendezvous
peer. That avoids ambiguous cookie bookkeeping because the current client event
for successful discovery reports the rendezvous node and cookie, but not the
namespace that produced the result. Additional configured tags are queried on
subsequent refreshes.
