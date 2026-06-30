# Identity key backup and rotation

`p2p-net` persists a libp2p node identity key at `identity_key_path`. That key determines the node's stable `PeerId`.

## Why it matters

- Keeping the same key keeps the same `PeerId` across restarts and upgrades.
- Losing the key creates a new `PeerId`; peers, allow lists, rendezvous registrations, relay policies, DNS records, and monitoring should treat the node as a new identity.
- Leaking the key lets someone else impersonate that node identity.

## Backup procedure

1. Stop the node cleanly.
2. Copy the file named by `identity_key_path` to an encrypted backup location.
3. Restrict permissions on the live key and backup so only the node operator account can read them.
4. Record the expected `PeerId` for the backup in deployment notes.
5. Restart the node and confirm the `PeerId` did not change.

Recommended backup targets are encrypted host backups, an encrypted secret manager, or encrypted offline media. Do not commit identity keys to source control or bake them into public container images.

## Restore procedure

1. Stop the node.
2. Restore the backed-up key to the configured `identity_key_path`.
3. Ensure file ownership and permissions match the node operator account.
4. Start the node.
5. Confirm the displayed/exported `PeerId` matches the recorded expected `PeerId`.

## Planned rotation procedure

Rotation intentionally changes the node's `PeerId`.

1. Schedule a maintenance window.
2. Stop the node.
3. Back up the old key and label it as retired.
4. Move the old key out of `identity_key_path` or update `identity_key_path` to a new empty path.
5. Start the node so a fresh identity key is created.
6. Record the new `PeerId`.
7. Update bootstrap lists, relay allow lists, rendezvous peer lists, DNS or `/dnsaddr` records, dashboards, and monitoring alerts that reference the old `PeerId`.
8. Keep the retired key only as long as required for audit or rollback. Store it encrypted.

## Emergency rotation after suspected compromise

1. Stop the compromised node immediately.
2. Remove the old `PeerId` from relay allow lists, bootstrap lists, rendezvous peers, dashboards, and monitoring trust rules.
3. Generate a new identity by starting with a new `identity_key_path`.
4. Publish the new `PeerId` through trusted deployment channels.
5. Treat any traffic signed or associated with the old `PeerId` after the compromise time as untrusted.

## Operator checks

- The identity key file should exist after first startup.
- The key file should not be world-readable.
- The same key should produce the same `PeerId` after restart.
- Rotation should be visible in operational notes because peers will see a new node identity.
