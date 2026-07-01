# Discovery namespaces

Application discovery uses deterministic namespaces so different applications,
networks, contacts, groups, and invite flows do not collide.

The derived namespace shape is:

```text
p2p-net/<network_id>/<app_id>/<tag_fingerprint>
```

By default, `<tag_fingerprint>` is a BLAKE3 hash derived from:

```text
p2p-net.discovery.namespace.v1 || network_id || app_id || raw_tag
```

Raw contact names, invite phrases, or human-readable rendezvous tags are not
published by default. For local debugging only, an operator may set
`privacy = "readable_unsafe"` together with `allow_readable_tags = true`.

## Configuration

```json
{
  "discovery": {
    "namespace": {
      "app_id": "hydra-msg",
      "tags": [
        "IAmJoeTheRealJoeYouWant213423432"
      ],
      "privacy": "hashed",
      "allow_readable_tags": false
    }
  }
}
```

When `discovery.namespace.tags` is empty, the node keeps using the operator
rendezvous namespace from `discovery.rendezvous.namespace`. This lets private
infrastructure-first deployments keep a simple operator-managed namespace while
apps opt into hashed per-contact/per-group namespaces when needed.

## Relationship to public fallback

This phase only defines the namespace model. Public bootstrap fallback and DHT
provider-record discovery are separate roadmap items. Once those are enabled,
the same derived namespace keys will be used for DHT provider records and
rendezvous discovery.
