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

## Production follow-through and reconnect priority

`wan-20260909-tcp-ready` exercised the startup fix in production nodes with
automatic public discovery, fresh namespaces and default 4001/4002 ports. Both
had one application peer at 26 seconds and again at the 60-second deadline,
but relay connections churned in between. Neither node recorded a direct
upgrade. Windows' established TCP connections used only listener ports 4001
and 4002 in the live sample. The Windows working set was approximately 104 MiB;
Ubuntu RSS was approximately 78 MiB. These short samples cannot establish
long-term memory stability.

This run exposed a separate reconnect defect: after a relay close, the app
peer's admission priority was removed, and its reconnect could be rejected at
the lower infrastructure ceiling (56 outgoing connections with the default
64 hard cap). Connection close now releases application keep-alive state but
preserves the bounded intended-peer admission cache. Explicit disconnect
suppression remains in the planner, and hard connection caps are unchanged.
A focused regression filled the infrastructure ceiling, simulated the
application keep-alive release, and verified that app admission still succeeds
while unrelated infrastructure is rejected.

## Retry interpretation

The configured per-peer cooldown gates locally initiating handler activations. Upstream
`libp2p-dcutr` 0.14.1 can retry three rounds within a handler without that cooldown.
Each round can dial several addresses. Neither the configured attempt budget
nor the current result counters should be interpreted as a count of individual
socket attempts. See [DCUtR policy](DCUTR_POLICY.md).

## Responder availability

The next production run, `wan-20260909-priority`, again ended at 60 seconds with
both app peers connected through relay and neither connected directly. The
target-peer rejection at the lower infrastructure cap did not recur. Ubuntu
instead reported a DCUtR `Unsupported` result after relay replacement.

The wrapper incorrectly applied local initiation cooldowns and budgets to
passive responders too. An asymmetric reconnect could therefore disable the
responder protocol while the other peer was still eligible to initiate.
Verified outbound relay-circuit handlers now remain able to respond; inbound
relay-circuit initiation still observes the cooldown and budget. Unverified
destinations remain disabled. Two focused regressions cover responder
availability after budget exhaustion and late verification without consuming
an initiation attempt. All ten focused DCUtR tests passed.

### Live results for `cab7769`

The normal release builds were run on Windows and Ubuntu, not a validation
suite. A per-target no-LTO build experiment failed on Windows because the
cached release dependencies require LTO; that experimental build was not used
for these tests. Ubuntu's corresponding experiment was stopped before the
normal release build.

- `wan-20260909-responder`: automatic public application discovery succeeded.
  Windows was the relay-circuit listener / DCUtR initiator. Both application
  peers remained connected at 54 seconds; Windows reported
  `AttemptsExceeded(3)`. Neither peer acquired a direct connection within 60
  seconds, and both processes reported failure. Windows' final app count fell
  to zero when Ubuntu shut down at its deadline, not during the earlier sample.
- `wan-20260909-responder-reverse`: a pinned free public relay diagnostic
  forced Windows to dial the relay circuit and act as the passive DCUtR
  responder. Ubuntu reported `AttemptsExceeded(3)`. Both app peers remained
  connected through relay at 60 seconds, with no direct connection in either
  raw event log. This pinned diagnostic does not establish automatic discovery
  acceptance. Windows' actual relay TCP socket used listener port 4001.

Neither run reported `Unsupported`. Both progressed to transport attempts:
Ubuntu recorded TCP connection refusals to the advertised Windows public
candidates, Windows recorded TCP timeouts to Ubuntu, and QUIC handshakes timed
out. These observations locate the remaining failure after DCUtR negotiation;
they do not identify every intervening NAT/filter or prove that no transport
implementation defect remains. The VPN and LAN policy were unchanged.

The reverse diagnostic also exposed an evidence-collection limitation: the
Windows relay endpoint event arrived before the other role's status file was
available. Its derived `target_relay_seen` flag stayed false although the raw
log proves the relay connection. Both raw logs were therefore checked directly
for the exact other PeerId and `relayed=false`; neither contained a direct
endpoint. Do not rely on that derived flag alone for early connections.

The Windows distribution was refreshed to the `cab7769` release build with an
updated checksum and an explicit failed-WAN-acceptance status. No successful
direct WAN upgrade, Android validation, or long-term memory-stability result
is claimed by this investigation.

## September 10: mapping control and earlier acceptance audit

Windows still used PIA `us-east` (`37.19.197.248`) and the Ubuntu VM remained
running on its approved bridge. No application-source change or full-validation
run was made for these controls.

Four standard STUN binding requests used the same Windows UDP socket, local
port 47801. Google (`74.125.250.129:19302`) observed public port 11711,
Cloudflare (`162.159.207.0:3478`) observed port 54892, and a final Google request
again observed 11711. These measurements establish destination-dependent
mapping on the tested Windows path. The corresponding Ubuntu requests all
returned `172.56.251.136:31636`; that sample does not establish its filtering
behavior. TCP STUN requests timed out on both machines and are inconclusive.

A separate ten-second raw UDP control used freshly STUN-observed endpoints
`37.19.197.248:48589` and `172.56.251.136:55374`. Each machine sent 100 packets
to the other endpoint and recorded zero received peer packets. Raw output is
under `target/wan-vm-build/exchange/udp-20260910-control/`. This bypasses the
application and DCUtR state machines, but uses Python sockets rather than the
app executable: per-executable firewall behavior and the exact filtering
location remain unisolated. It is not a DCUtR acceptance result and does not
prove mathematical impossibility of every traversal strategy.

The earlier commit `3dbbdda` is titled "libp2p dicutr & webrtc hole punching
success". Its default DCUtR parent probe spawned both child processes on the
same host, while its candidate filter allowed private non-loopback addresses.
That harness did not enforce the current distinct-host, WAN-only requirement.
The commit title alone therefore cannot serve as a known-good WAN regression
baseline; this audit does not assert which address a historical successful run
actually used without its raw endpoint evidence.

The requested direct Windows-VPN / Ubuntu-WAN result remains unachieved.

## September 10: TCP retry overlap correction

A prior working binary is not required to develop a correction. The next
experiment used the observed fast TCP refusals and overlapping Windows
connects as its starting evidence, without changing the VPN or enabling LAN.

First, listener-role TCP connects were given an eight-second window for
retrying early refusal/address-in-use errors against the same advertised
endpoint, with a 250 ms delay. `wan-20260910-tcp-window` still failed at 60
seconds. Linux reported `AttemptsExceeded(3)` at 26 seconds. Windows' next
round could still start while its preceding ten-second connect was pending:
target TCP dials were constructed around 2, 10 and 19 seconds, and target
`10048` errors remained. Holding the listener side's refusal alone was
insufficient.

Commit `7649d2c` additionally serializes pending reused-port TCP connects to
the same destination. Cancellation releases the waiting slot; established
streams are never shared between Noise/Yamux handshakes. Weak references are
reclaimed on subsequent lookups rather than retaining endpoint history.
Different destinations and explicit new-port requests remain independent.
Ordinary dialer-role refusals are not retried. The eight focused transport
tests passed, including an actual refused socket followed by successful
listener-port reuse, cancellation, independent endpoints, and stale-entry
cleanup. These regressions are not WAN acceptance.

`wan-20260910-tcp-gate` ran the updated production code on Windows and Ubuntu
through the same pinned free public relay, with Ubuntu initiating DCUtR.
Both connected as application peers through relay. The target Windows TCP
collision errors did not recur, but TCP/QUIC direct attempts still failed and
Linux again reported `AttemptsExceeded(3)`. Neither raw log contains a direct
endpoint for the other PeerId, and both processes failed their 60-second
deadline. Unrelated infrastructure peers still produced some address-in-use
errors; this result does not claim to eliminate every such error globally.
Windows' final app count dropped when Ubuntu exited at its deadline.

Both runs were pinned-relay diagnostics, not automatic-discovery acceptance.
No full-validation suite was run. The normal Windows app was rebuilt and
copied to `dist/windows`, with its previous binary and metadata preserved in
`target/wan-vm-build/dist-before-7649d2c`. Its manifest explicitly records failed
direct-WAN acceptance. Direct Windows-VPN / Ubuntu-WAN connectivity remains
unresolved despite the narrower TCP collision correction.
