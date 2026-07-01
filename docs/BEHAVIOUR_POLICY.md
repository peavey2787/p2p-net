# Profile-driven behaviour policy

libp2p behaviour construction consumes the central `ResolvedNodeConfig` capability policy instead of re-deciding role behaviour inside the stack builder.

## Runtime flow

```text
NodeConfig
  -> EnvironmentReport
  -> resolve_node_config(...)
  -> ResolvedNodeConfig
  -> with_resolved_capabilities_applied(...)
  -> build_swarm(..., resolved_config)
  -> build_behaviour(..., resolved_config)
```

The raw config remains the user-facing input. The resolved config is the runtime policy object.

## Kademlia policy

Kademlia is still present in the fixed `MeshBehaviour` type, but its mode is no longer hard-coded to server mode.

- `full`, `relay`, `mediator`, `rendezvous`, and `bootstrap` profiles use Kademlia server mode.
- `lite` and `mobile_lite` profiles use Kademlia client mode.
- This prevents NAT/CGNAT/mobile nodes from accidentally acting like infrastructure nodes.

## Relay and rendezvous policy

Relay server and rendezvous server behaviours are toggled only when both conditions are true:

1. the central resolver enables the capability, and
2. the effective runtime config still enables the matching service.

That keeps profile policy centralized while preserving config validation and feature-specific settings.

## Transport labels

The active transport/capability list now reports resolved behaviour intent, including:

- `kademlia-client`
- `kademlia-server`
- `relay-client`
- `relay-server`
- `rendezvous-client`
- `rendezvous-server`

This keeps snapshots and dashboard output aligned with the actual role policy.

## DCUtR policy

`DcutrPolicy` uses the same resolved behaviour path. The DCUtR behaviour is installed only when the resolved behaviour set enables DCUtR. Disabling `dcutr.enabled` removes the behaviour while keeping relay-client fallback available. See `docs/DCUTR_POLICY.md` for fallback and retry semantics.
