# Security policy

## Production security model

`p2p-net` treats all network input, discovery data, DNS/DNSADDR answers, peer metadata, relay requests, WebRTC/STUN traffic, and application envelopes as untrusted.

Release validation is fail-closed around a committed dependency graph and exact Rust/tool versions. The normal validation launchers do not regenerate `Cargo.lock`. GitHub Actions uses read-only repository permissions and immutable action SHAs. A separate scheduled security workflow runs hostile/soak tests and bounded libFuzzer campaigns.

### Application-message authenticity and confidentiality

Gossipsub messages use signed authenticity. On receive, the authenticated Gossipsub author (`message.source`) is authoritative. `AppMessage.source_peer_id` must match that author and the envelope topic must match the authenticated outer Gossipsub topic. The immediate forwarding peer (`propagation_source`) is used only for transport accounting and Gossipsub validation reporting.

`send_message(peer_id, topic, payload)` is **addressed delivery over Gossipsub, not a confidential point-to-point channel**. Other subscribed mesh peers may carry/observe its payload while forwarding it. Applications that require confidentiality must encrypt payloads end-to-end or build/use a direct encrypted stream/request-response protocol.

### Identity keys

Desktop identity creation is create-if-absent and race-safe, refuses symlink/non-regular-file secret paths, refuses secret overwrite, and tightens secret permissions. Unix secrets are forced to mode `0600`. Windows secrets have inherited ACLs removed and access granted to the creating account. Embedders should use `NodeStorage` to place identities in platform-private Keychain/Keystore/DPAPI-backed storage where available.

### Resource exhaustion

Production defaults retain full-node capabilities. Resource hardening is implemented with bounded historical/replay/reputation/transport bookkeeping, bounded message/config/DNS processing, connection/relay policy, WebRTC handshake cleanup, and coalesced persistence/observability rather than by disabling full Kademlia, relay, WebRTC, or normal protocol cadences.

## Dependency and toolchain policy

- Rust is pinned by `rust-toolchain.toml`.
- `Cargo.lock` is committed and validated with `--locked`.
- Manifest-backed security floors prevent lock regeneration from reintroducing patched `event-listener` (<5.4.2, RUSTSEC-2026-0221) or `h2` (<0.4.16, RUSTSEC-2026-0258) releases.
- `cargo-audit` and `cargo-deny` versions are pinned by the root validation launchers.
- Third-party GitHub Actions are pinned to immutable commit SHAs.
- Dependency refresh is an explicit maintenance operation followed by full validation; production CI never refreshes dependencies implicitly.
- Known-unsound dependencies are release blockers. Unmaintained dependencies are also release blockers except for the two exact temporary transitive exceptions below. No blanket unmaintained allowance is permitted.

### Temporary unmaintained transitive debt

Two RustSec unmaintained advisories are temporarily allowlisted because their maintained upstream exit paths are not yet available through the supported released dependency families:

- `RUSTSEC-2025-0141` / `bincode 1.3.3`: currently enters through `webrtc-dtls 0.11` / `webrtc 0.12`. Migrate with the matching released `libp2p-webrtc 0.10+` family that carries the upstream WebRTC 0.17 negotiation changes. At that migration, require `cargo tree -i bincode` to return no package and add a manifest-backed `rkyv >=0.8.18, <0.9` security floor if `rkyv` remains in the resolved WebRTC stack.
- `RUSTSEC-2024-0436` / `paste 1.0.15`: currently enters through `if-watch` and the Linux netlink 0.8 family. Remove it when a maintained `if-watch` release adopts `netlink-packet-core 0.9+`; validate Linux interface churn, address add/remove, TCP/QUIC rebinding, and long-running network-change behavior. Require `cargo tree -i paste` to return no package.

Both advisory IDs are explicitly ignored rather than treating all unmaintained crates as warnings. `cargo-audit` denies every other unmaintained advisory, while `cargo-deny` uses `unmaintained = "all"` and fails if either temporary ignore becomes unused. Therefore once an upstream migration removes either crate, validation intentionally fails until its stale exception is deleted; after both exceptions are removed, all unmaintained dependencies are hard release failures automatically.

## Reporting security issues

Do not publish suspected vulnerabilities in a public issue before maintainers have had a reasonable opportunity to investigate. Prefer the repository's private GitHub Security Advisory reporting channel when enabled. Include affected commit/version, reproduction steps, impact, and any proposed mitigation.
