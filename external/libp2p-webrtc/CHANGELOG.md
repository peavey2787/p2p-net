## p2p-net 0.57 rebase

- Rebase the hardened transport onto the rust-libp2p 0.57 generation: `libp2p-core` 0.44, `libp2p-identity` 0.3, `libp2p-noise` 0.47, `libp2p-webrtc-utils` 0.5, `webrtc` 0.17, `stun` 0.17, and `rand` 0.10.
- Preserve p2p-net hardening for bounded/expiring half-open UDP state, failed/cancelled connection cleanup, setup timeout, close-on-drop, constrained SRTP profiles, fuzzing support, and smoke tests.
- Retain PEM certificate serialization support for stable WebRTC-direct certificate persistence.
- Create the dialer's negotiated Noise data channel before its SDP offer. `webrtc` 0.17 starts SCTP only when both local and remote descriptions carry an SCTP port, and an offer created before any data channel has no `m=application` section, so outbound dials never opened the Noise channel and timed out.

## 0.9.0-alpha.1

- Bump `webrtc` dependency to `0.12.0`.
  See [PR 5448](https://github.com/libp2p/rust-libp2p/pull/5448).

## 0.9.0-alpha

<!-- Update to libp2p-core v0.43.0 -->

## 0.8.0-alpha

- Implement refactored `Transport`.
  See [PR 4568](https://github.com/libp2p/rust-libp2p/pull/4568)

## 0.7.1-alpha

- Bump `libp2p-webrtc-utils` dependency to `0.2.0`.
  See [PR 5118](https://github.com/libp2p/rust-libp2p/pull/5118).

## 0.7.0-alpha

- Bump version in order to publish a new version dependent on latest `libp2p-core`.
  See [PR 4959](https://github.com/libp2p/rust-libp2p/pull/4959).

## 0.6.1-alpha

- Move common dependencies to `libp2p-webrtc-utils` crate.
  See [PR 4248].

[PR 4248]: https://github.com/libp2p/rust-libp2p/pull/4248

## 0.6.0-alpha

- Update `webrtc` dependency to `v0.8.0`.
  See [PR 4099].

[PR 4099]: https://github.com/libp2p/rust-libp2p/pull/4099

## 0.5.0-alpha

- Raise MSRV to 1.65.
  See [PR 3715].

[PR 3715]: https://github.com/libp2p/rust-libp2p/pull/3715

## 0.4.0-alpha.4

- Make `Fingerprint` type public. See [PR 3648].

[PR 3648]: https://github.com/libp2p/rust-libp2p/pull/3648

## 0.4.0-alpha.3

- Gracefully handle `ConnectionReset` error on individual connections, avoiding shutdown of the entire listener upon disconnect of a single client.
  See [PR 3575].

- Migrate from `prost` to `quick-protobuf`. This removes `protoc` dependency. See [PR 3312].

[PR 3575]: https://github.com/libp2p/rust-libp2p/pull/3575
[PR 3312]: https://github.com/libp2p/rust-libp2p/pull/3312

## 0.4.0-alpha.2

- Update to `libp2p-noise` `v0.42.0`.

- Update to `libp2p-core` `v0.39.0`.

## 0.4.0-alpha

- Initial alpha release.
