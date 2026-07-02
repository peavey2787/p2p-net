# Production discovery and mediation fleet

A production deployment should run a small owned fleet before relying on public recovery paths. The goal is not to make every client an infrastructure node. The goal is to provide stable discovery, address exchange, relay reservation, and DCUtR mediation for clients that cannot accept inbound connections.

## Recommended roles

| Role | Purpose | Typical profile | Notes |
|---|---|---|---|
| Bootstrap | Stable first dial targets | `bootstrap` | Publish stable DNS or `/dnsaddr` records and keep PeerId stable. |
| Rendezvous | Application namespace registration and discovery | `rendezvous` | Enable rendezvous client/server and use hashed app namespaces. |
| Mediator | Relay reservation plus DCUtR coordination | `mediator` | Prefer allow-listed authenticated peers for private deployments. |
| Relay | Generic Circuit Relay v2 service | `relay` | Use explicit capacity, rate, schedule, and ACL settings. |
| Client | Application node | `lite`, `mobile_lite`, or `auto` | Dial owned infra first, keep relay fallback, and use the peer book. |

A small deployment can combine bootstrap and rendezvous roles on the same host, but capacity planning is easier when relay/mediator traffic is isolated from discovery traffic.

## Identity and addressing

Keep every infrastructure node's identity key durable and backed up. PeerId changes break pinned multiaddrs and cached trust relationships. Use DNS names or `/dnsaddr` records for host mobility, but keep `/p2p/<PeerId>` in every configured peer address so the transport authenticates the expected peer.

Recommended address forms:

```text
/dnsaddr/bootstrap.example.net/p2p/<owned-bootstrap-peer-id>
/dnsaddr/rendezvous.example.net/p2p/<owned-rendezvous-peer-id>
/dnsaddr/relay.example.net/p2p/<owned-relay-peer-id>
```

Use an internal or self-hosted DoH endpoint through `dnsaddr.doh_endpoint` when `/dnsaddr` lookup privacy or third-party dependency control matters.

## Capacity and abuse controls

For relay and mediator nodes:

- Set `relay.max_reservations`, `relay.max_reservations_per_peer`, `relay.max_circuits`, and `relay.max_circuits_per_peer` to measured capacity.
- Set byte and duration limits for circuits.
- Use `relay.access = "allow_list"` with `relay.allow_peers` for private applications.
- Keep `mediator.require_authenticated_peers = true` only when the relay allow-list is populated.
- Monitor denied reservations, denied circuits, active circuits, bytes forwarded, and abuse rate-limit events.

## Availability layout

Run at least two bootstrap/rendezvous targets in different failure domains. Run relays close to expected users when latency matters. For private-infrastructure-first fleets, disable public fallback; for consumer fleets, keep fallback-only mode but monitor when public infrastructure is used.

## Operational checklist

- Back up infrastructure identity keys and document restore steps.
- Validate configs before deployment with the repository full-validation script.
- Publish stable DNS or `/dnsaddr` records for infrastructure hosts.
- Keep peer-cache storage durable on clients.
- Monitor public fallback participation; it should be expected for consumer first-launch flows and rare after private/cached infrastructure has taken over.
- Run hostile-network tests against staging relays before exposing them to real clients.
