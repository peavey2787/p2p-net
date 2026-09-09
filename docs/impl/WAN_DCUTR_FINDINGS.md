# WAN DCUtR investigation — September 9, 2026

Direct WAN DCUtR has **not** passed acceptance on the tested Windows-VPN /
Ubuntu-regular-Internet pair. Public relay connectivity is not direct success.
No full-validation suite was run.

## Conditions

- Windows remained on PIA `us-east`; Ubuntu used its approved bridged regular
  Internet connection. No VPN location, forwarding or firewall changes.
- Production probes used fresh identities/namespaces and disabled LAN discovery.
- Each live attempt stopped at 60 seconds without a direct upgrade.
- The isolated control used a free public relay, stock TCP/Noise/Yamux, Identify
  and DCUtR. The shared directory carried identities/readiness, not LAN dial
  addresses. It is a diagnostic, not production-node acceptance.

## Observations

1. Reversing production relay/DCUtR roles still produced one application peer
   on each machine via relay at 60 seconds, with zero direct upgrades. Windows
   reported TCP timeouts, without the previous address-in-use error in that run.
2. The first isolated TCP control was **invalid as a network control**: it dialed
   before its wildcard TCP listener registered a reusable port. Windows' relay
   connection used ephemeral local port 57992 instead of listener port 47501.
   Its negative result must not be used to blame the network.
3. The corrected TCP control waited for listener registration. Live socket tables
   verified relay connections from Windows' configured 47501 and Ubuntu's 47601.
   Both established their relay circuit, but neither established a direct
   connection within 60 seconds. Ubuntu received TCP refusals. Its fast retries
   overlapped Windows' still-pending first TCP connect, producing address-in-use
   errors there before the first dial timed out. This reproduces a failure with
   no application planner, DHT traffic, or QUIC transport involved. It does not
   prove every upstream transport/NAT issue has been ruled out.

Local raw evidence is under `target/wan-vm-build/exchange/`:
`wan-20260909-reversed`, `tcp-control-20260909-a` (invalid control), and
`tcp-control-20260909-b` (corrected control). These generated files are not
version-controlled.

## TCP startup fix

Production also issues bootstrap dials before polling its TCP listeners.
`ReadyTcpTransport` now delays reused-port dials while a matching wildcard
listener awaits interface registration. It uses asynchronous notifications,
preserves native address validation, and is shared by native TCP and WebSocket.
Outbound-only dialing and explicit new-port requests are not delayed. Listener
removal wakes waiters; address expiry clears readiness.

Three focused socket regressions passed: early wildcard reuse, removal of a
pending listener, and outbound-only dialing. The tests use local sockets only
and do **not** establish WAN traversal success. Earlier pinned-relay production
captures already showed correct listener-port reuse, so this startup bug is not
claimed as the sole explanation for all failures.

## Retry interpretation

The configured per-peer cooldown gates p2p-net handler activations. Upstream
`libp2p-dcutr` 0.14.1 can retry three rounds within a handler without that cooldown.
Each round can dial several addresses. Neither the configured attempt budget
nor the current result counters should be interpreted as a count of individual
socket attempts. See [DCUtR policy](DCUTR_POLICY.md).
