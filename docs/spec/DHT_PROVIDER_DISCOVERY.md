# DHT Provider Discovery Specification

## Purpose

DHT provider discovery is the fallback discovery layer for application namespaces. It lets nodes announce and search for hashed namespace keys through Kademlia provider records when operator rendezvous peers are unavailable.

The public application API remains the six primitives:

- `connect_peer`
- `disconnect_peer`
- `send_message`
- `broadcast`
- `subscribe`
- `get_peers`

DHT provider discovery is internal plumbing behind those primitives.

## Priority order

Nodes should prefer owned infrastructure first:

1. operator bootstrap peers
2. operator rendezvous peers
3. operator mediator/relay peers
4. healthy peer cache
5. explicitly configured public bootstrap or relay fallback
6. DHT provider lookups for hashed discovery namespaces

Public infrastructure must be opt-in through `discovery.public_bootstrap`. DHT provider discovery may be enabled by default, but it only becomes useful after the node has joined a Kademlia routing layer through configured, cached, or fallback bootstrap peers.

## Namespace keys

DHT provider keys use the same derived discovery namespaces as rendezvous discovery:

```text
p2p-net/<network_id>/<app_id>/<tag_hash>
```

Raw contact tags must not be published by default. The namespace model hashes app tags unless the operator explicitly enables readable unsafe tags.

## Announcement

When `discovery.dht.enabled` and `discovery.dht.announce` are true, startup calls Kademlia `start_providing(namespace_key)` for each derived namespace, bounded by `max_namespaces_per_refresh`.

## Discovery

When `discovery.dht.enabled` and `discovery.dht.discover` are true, startup calls Kademlia `get_providers(namespace_key)` for each derived namespace only when there are no configured rendezvous peers by default.

Operators can set:

```json
{
  "discover_with_rendezvous_peers": true
}
```

to run DHT provider lookup alongside rendezvous discovery.

## Runtime results

Provider lookup results update internal DHT provider state and observability counters. Full peer-book-backed `get_peers()` integration is the next roadmap phase.
