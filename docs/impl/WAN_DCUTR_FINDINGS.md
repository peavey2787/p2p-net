# WAN DCUtR investigation — September 9–11, 2026

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

## September 10: measured mapping/filtering mismatch

The new reproducible diagnostic is
[`scripts/diagnostics/nat_behavior.py`](../../scripts/diagnostics/nat_behavior.py):

```sh
python scripts/diagnostics/nat_behavior.py --tcp-mapping
```

It uses standard STUN requests to `stun.voipgate.com:3478`, the default public
service in [Pion's NAT diagnostic](https://github.com/pion/stun/blob/main/cmd/stun-nat-behaviour/README.md).
The service advertised primary `185.125.180.70:3478` and alternate
`185.125.180.71:3479`. Requests are restricted to these two service addresses
and their two ports; no port scan, forwarding, firewall or VPN change occurs.
Socket operations share a 60-second budget; DNS uses the OS resolver.

Both hosts were tested, first with a UDP diagnostic and then with the final
combined UDP/TCP implementation. Windows remained on PIA `us-east`, public
`37.19.197.248`; Ubuntu remained on regular Internet, public `172.56.251.136`.
The final generated results are in
`target/wan-vm-build/exchange/nat-20260910-complete/{windows,linux}.json`.

| Measurement | Windows VPN | Ubuntu regular Internet |
| --- | --- | --- |
| UDP mapping | Address-and-port-dependent | Endpoint-independent in the sample |
| UDP filtering | Address-and-port-dependent | Address-and-port-dependent |
| TCP mapping from one local listening port | Public ports 44097, 62852, 10146 for three destinations | Public port 18278 for all three destinations |

In the UDP mapping check, Windows used one socket but obtained public ports
52709, 37272 and 11939 for the three service destinations. Ubuntu's corresponding
two-address check retained public port 1305. Mapping and filtering checks use
separate fresh sockets, so mapping requests do not accidentally open the return
paths before filtering is measured.

The filtering result is not based solely on silence from an unverified server:

- The diagnostic verifies response transaction IDs, source addresses, framing
  and `RESPONSE-ORIGIN`, and rejects a missing/private/unusable `OTHER-ADDRESS`.
- After alternate-source responses time out, it confirms direct binding replies
  from all alternate endpoints. This addresses the false-classification issue
  in [reported RFC 5780 erratum 7971](https://www.rfc-editor.org/errata/eid7971).
- On Ubuntu, where the mapped port remains unchanged, it then repeats both
  CHANGE-REQUESTs after explicitly opening the alternate return paths. Both
  replies arrive from the requested alternate endpoints. This verifies that the
  service supports the operation and that the earlier filtering result was not
  simply a server silently ignoring the request.
- Five small offline codec/guard checks pass, including rejection of a server
  that responds from the original endpoint to a change-source request.

This identifies a concrete obstacle for the current UDP strategy: Windows'
public port observed by a relay/STUN server is not generally the source port
used toward Ubuntu, while Ubuntu only accepts traffic from the exact endpoint
it contacted. Merely synchronizing dials to those third-party-observed ports
does not resolve that mismatch. Increasing the retry count does not make those
ports destination-independent.

These are Python-socket path measurements, not measurements made inside the
app's QUIC socket. They do not locate every filtering device, characterize TCP
filtering, rule out every remaining transport defect, or prove every conceivable
traversal strategy impossible. They do provide evidence beyond an application
timeout; TCP mapping is now measured too, unlike the earlier inconclusive
Google/Cloudflare TCP STUN controls.

### Fresh production acceptance after the controls

`wan-20260910-nat-confirmed` ran the existing `7649d2c` production build on both
hosts with automatic public discovery, a fresh identity/namespace, LAN disabled,
and no manually exchanged relay address. Both app peers were connected through
relay at 32–33 seconds. Windows reported `AttemptsExceeded(3)`; both processes
failed their 60-second deadlines. Neither complete raw log contains a direct
endpoint for the exact other PeerId, and both success counters remain zero.
Ubuntu's last status still showed the app connection; Windows' final app count
dropped at the other process's shutdown. That final sample does not establish
an earlier spontaneous disconnect.

No application binary was changed by this diagnostic-only follow-up. No full
validation was run. Direct WAN DCUtR remains **unachieved**; neither relay
connectivity nor these network measurements are reported as direct success.

## September 11: remove the frozen DCUtR candidate quota

Commit `efb9d19` corrects a real address-refresh defect: the wrapper permanently
retained the first eight public TCP and eight QUIC candidates, rejecting all
later distinct observations. Consequently, upstream's existing 20-entry LRU
could not refresh after that quota filled. The duplicate lifetime cache is
removed; the WAN/transport filter and app-peer admission rules remain. Fresh
observations now reach upstream's bounded shared cache. This no longer imposes
separate per-transport quotas. Existing handler snapshots are unchanged.

The focused regression exercised 200 TCP/QUIC mapping observations and verified
that fresh public addresses remain eligible while private addresses remain
rejected with LAN disabled. It passed. Windows release app/probe builds and
Ubuntu's release probe build completed; no full validation suite was run.

`wan-20260911-candidate-refresh` then tested those builds using automatic public
discovery, fresh identities, LAN disabled, unchanged Windows PIA `us-east`, and
Ubuntu's regular-Internet bridge. Both had their target application peer through
relay at 27–28 seconds. Neither raw log contains a direct endpoint for the other
PeerId. Both failed the hard 60-second deadline, with zero DCUtR successes; no
terminal DCUtR result was emitted before that deadline in this run.

An additional Ubuntu capture on `enp0s3` was limited to Windows' WAN IP and
TCP/UDP app port 4001, recording packet headers rather than payloads. It recorded
1,179 packets with zero kernel drops. These include 227 incoming TCP resets
from Windows' advertised public endpoints in response to Ubuntu's outgoing
SYNs, but no incoming TCP SYN or UDP packet from that IP within the capture.
The capture establishes that those direct TCP dials left Ubuntu and met resets;
it does not identify whether Windows or an intervening device generated them.

Windows PktMon access was denied even from the available escalated tool session.
An administrator/UAC-approved targeted Windows capture is the next isolating
check for the reset origin; no VPN, firewall or routing change is needed for
that capture. The Ubuntu capture stopped automatically after 65 seconds.

`dist/windows` now contains the rebuilt `efb9d19` normal app. Its manifest and
checksum are updated and explicitly record failed direct-WAN acceptance. The
prior executable and metadata are preserved in
`target/wan-vm-build/dist-before-efb9d19`; local configuration, identities and
peer caches were not replaced. This fixes the frozen-candidate defect, not the
still-unresolved end-to-end direct connection.
