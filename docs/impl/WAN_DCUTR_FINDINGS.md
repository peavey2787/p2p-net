# WAN DCUtR investigation — September 9–13, 2026

Direct WAN DCUtR now passes acceptance on the tested Windows-VPN /
Ubuntu-regular-Internet pair. The September 13 results at the end of this file
supersede the earlier unresolved status. Public relay connectivity alone was
not counted as success; both peers had to record a later direct connection and
a successful DCUtR result. No full-validation suite was run.

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

## September 12: simultaneous administrator Windows / Ubuntu captures

The user approved a UAC-elevated Windows packet-header capture. The first helper
stopped without changing filters because its empty-filter check expected
"No filters" but Windows returned "Packet Filters: None". After correcting that
check, `wan-20260912-windows-capture-b` ran the existing `efb9d19` release builds
with automatic public discovery, fresh identities, LAN disabled and the same
Windows VPN / Ubuntu regular-Internet paths.

Windows PktMon captured only TCP SYN/RST packets involving Ubuntu's public
`172.56.251.136` and app port 4001, across all components, limited to 54 bytes
per record and an 8 MB circular log. It verified that no other capture or
filters existed before starting. Ubuntu simultaneously captured headers for
Windows' public `37.19.197.248` on app TCP/UDP port 4001.

Results:

- Windows recorded 36 outgoing SYN observations across four component/edge
  appearances: **nine SYN transmissions**, not 36 distinct packets. They used
  local `10.29.18.238:4001` toward `172.56.251.136:21429`, the public TCP mapping
  reported in Ubuntu's Identify log. Component 1 is the **PIA Tunnel**, and the
  same SYNs are visible there. There were no matching receive records or reset
  records in Windows' filtered capture. PktMon reported no lost events.
- Ubuntu recorded 25 outgoing TCP SYNs and 25 incoming TCP resets from the
  advertised Windows public ports. For example, a SYN to port 30580 with
  sequence 3807317937 was followed by a reset acknowledging 3807317938. Windows'
  Identify log contains that advertised port. Ubuntu recorded no incoming SYN
  or UDP packet from Windows' public IP. Its capture reported 270 packets and
  zero kernel drops.
- Both application peers stayed connected through relay at the final 60-second
  sample, with zero direct successes. Windows emitted `AttemptsExceeded(3)`.
  Neither raw application log contains a direct endpoint for the other PeerId;
  both processes failed the 60-second acceptance deadline.

The outgoing SYNs provide a positive visibility check at Windows' VPN interface:
this is not a conclusion drawn from an entirely empty Windows trace. Ubuntu's
resets are not visible as resets emitted by the captured Windows TCP path, and
Windows' SYNs are not visible at Ubuntu's interface. This points to rejection
or loss in the intervening VPN/NAT path, rather than an absent application dial.
The captures do **not** identify the exact generating hop or rule out filtering
inside the VPN client before packets reach the observed Windows interfaces.

The Windows helper stopped capture, removed its temporary filters, and converted
the ETL into `windows-peer.txt`. Ubuntu's capture stopped automatically. No VPN,
firewall or routing setting was changed. Raw evidence and the administrator
transcript are under
`target/wan-vm-build/exchange/wan-20260912-windows-capture-b/`. No application
source/binary change or full-validation run was made in this capture follow-up.

## September 12: recover guest execution and retest both WAN nodes

The previous `tcp-20260912-allports` attempt is invalid: Ubuntu never started
because VirtualBox had exhausted its guest sessions. It is not evidence of a
two-node connection failure. On resuming, the VM was powered off. It was started
headlessly, the approved transient source share restored, and guest execution
verified before either new test. Windows stayed on PIA `us-east`; its current
public address was `212.56.54.127`. Ubuntu remained on the approved regular-
Internet bridge and reported `172.56.251.136`.

`scripts/diagnostics/invoke_guest_command.ps1` now provides an attached,
time-bounded `guestcontrol run` invocation instead of detached `start` calls.
It accepts caller-supplied credentials without persisting them. A successful
guest command, both failed live probes, and an intentional one-second timeout
each left **zero active guest sessions**. The timeout returned after 1.2 seconds.
This repairs diagnostic session accumulation; it is not a NAT-traversal fix.

Two valid live attempts used the existing `efb9d19` binaries:

- `wan-20260912-attached`: both production nodes ran for 60 seconds with fresh
  identities, automatic public discovery and LAN disabled. Their relayed
  application circuit appeared at about 19 seconds. Windows emitted
  `AttemptsExceeded(3)` at 46 seconds. Both recorded zero DCUtR successes and no
  direct target endpoint. The target's errors were TCP connect-window and QUIC
  handshake timeouts, not connection-limit rejections. Windows' final app count
  was zero after the other process reached its deadline; the earlier relay
  connection is present in both logs.
- `tcp-20260912-attached`: both stock TCP/DCUtR control processes ran for
  60 seconds, established the intended public relayed circuit and used only
  relay-observed public TCP candidates. Ubuntu received connection refusals
  dialing Windows' observed port 60125. Windows' first dial to Ubuntu's observed
  port 35093 timed out; overlapping stock retries returned `AddrInUse`. That
  overlap is already mitigated in the production TCP wrapper, which this
  deliberately stock control does not use. Neither control reported a direct
  endpoint or DCUtR success. This is diagnostic evidence, not production
  acceptance or proof that every hole-punching strategy must fail.

Raw status and logs remain in the named directories under
`target/wan-vm-build/exchange/`. No VPN location, firewall, routing or production
transport change was made, and no full validation was run. Direct WAN DCUtR
acceptance was still **unachieved at that point**; the September 13 result
below supersedes this intermediate conclusion.

## September 13: automatic discovery and direct WAN acceptance

The final correction separates quick application discovery from long-lived
routing-table maintenance. Inserting public bootstrap seeds no longer triggers
rust-libp2p's automatic full-bucket bootstrap during startup, and p2p-net no
longer explicitly launches a duplicate eager bootstrap. Application provider
announce/lookups route from the known seeds immediately. The configured
periodic bootstrap remains enabled for routing-table maintenance, and the Full
role remains a Kademlia server. Provider discovery again uses three independent
keys, with a replication factor of five per query and bounded startup retries
at 5, 10, 20, and 35 seconds.

The connection fix also removes DCUtR's relay-direction dependency. After an
inbound relayed connection proves the exact application namespace through
Identify, the recipient opens one bounded reciprocal circuit to the same app
peer. That gives DCUtR both NAT orientations without allowing random relayed
peers into the application set or namespace-filtering the public relay itself.

Two consecutive fresh-identity runs used production `start_node(NodeConfig)`
through `live_dcutr_process_probe`, automatic public-DHT discovery, the free
public relay at `38.146.27.201`, and LAN discovery disabled. The shared folder
carried only status observations; neither node consumed the other node's ID or
addresses for dialing. Windows stayed behind PIA `us-east` at
`37.19.197.182`; Ubuntu used regular Internet at `172.56.251.136`.

| Run | Windows direct | Ubuntu direct | Relay seen first | DCUtR result |
| --- | ---: | ---: | --- | --- |
| `wan-20260913-auto-no-bootstrap-f` | 16 s | 39 s | both peers | `Ok` on both peers |
| `wan-20260913-auto-no-bootstrap-g` | 13 s | 33 s | both peers | `Ok` on both peers |

Every final status contained one exact application peer,
`target_relay_seen=true`, `target_direct_after_relay=true`, one DCUtR attempt,
zero DCUtR failures, and one DCUtR success. These are direct WAN QUIC
connections established after the relayed circuit, not LAN or relay-only
successes. Both runs completed inside each process's hard 60-second deadline.
Raw generated statuses remain under `target/wan-vm-build/exchange/` and are not
version-controlled.
