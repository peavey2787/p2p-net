# Roadmap: consumer public-network auto-connect

This roadmap replaces the completed hygiene roadmap. The goal is to make the normal app experience consumer-friendly:

- two fresh app installs can join the public app network with default settings;
- public fallback is on by default for normal app mode;
- private-infrastructure-first remains an advanced/operator mode;
- manual bootstrap, rendezvous, relay, and DNSADDR peers remain optional power-user controls;
- discovered peers may be auto-connected at the network layer, but must not be auto-trusted as contacts.

## Current gap

The current default has public bootstrap fallback, but that is not enough for the expected UX. Public bootstrap nodes can help a node reach the wider DHT, but they do not by themselves provide:

1. public app rendezvous candidates;
2. public relay/mediator candidates for NAT-to-NAT paths;
3. network-layer auto-dial of app-namespace-discovered peers;
4. clear status that distinguishes known/discovered peers from connected peers;
5. a first-run consumer profile distinct from private/operator infrastructure mode.

## Design rules

Every step must preserve these invariants:

- **Auto-connect is not auto-trust.** A discovered peer may be dialed, but it must not become a trusted contact without QR, join code, invite acceptance, safety-number verification, or an equivalent explicit app action.
- **Do not fake infrastructure.** The shared library may define slots and defaults for public infrastructure, but it must not pretend a relay/rendezvous fleet exists when no real endpoints are configured.
- **Power users stay in control.** Operators can disable public fallback, supply private bootstrap peers, supply private rendezvous/relay peers, and run private-infrastructure-only mode.
- **Public fallback remains visible.** Snapshots, docs, and examples must make it clear when public bootstrap/rendezvous/relay fallback is being used.
- **No SRP regression.** Consumer-default work must not collapse startup, discovery, relay, and contact-trust logic back into god files.

Run the canonical validation command before marking any step complete:

```powershell
.\qa\ci\run-full-validation.ps1
```

## Step 1 — Add consumer public-network policy defaults

Status: implemented; pending full validation.

Goal: make the default config model explicitly describe the consumer public-network behavior instead of treating public fallback as only bootstrap seeds.

Scope:

- Extend `discovery.public_bootstrap` to include public app rendezvous candidates in addition to bootstrap and relay candidates.
- Add an explicit `auto_connect_discovered_peers` policy flag that defaults to `true` for normal app mode.
- Keep private/operator helper config disabling public fallback and public auto-connect.
- Enable the rendezvous client in normal app defaults while keeping the rendezvous server disabled.
- Keep DHT provider discovery enabled even when rendezvous candidates are present.
- Add parsed accessors and best-effort startup plumbing for public rendezvous candidates without adding final UI counters yet.
- Update docs/examples/tests so normal app mode is public-network-first, and private-infrastructure-first is clearly advanced/operator mode.

Acceptance criteria:

- `NodeConfig::default()` has public fallback mode `fallback_only`.
- `NodeConfig::default()` enables rendezvous client behavior but not rendezvous server behavior.
- `DiscoveryConfig::default().dht.should_discover(1)` is true so DHT provider discovery can run alongside public rendezvous.
- `PublicBootstrapConfig::default().auto_connect_discovered_peers` is true.
- `PublicBootstrapConfig::private_infrastructure_only().auto_connect_discovered_peers` is false.
- Existing public relay defaults remain honest: no relay endpoints are shipped unless real endpoints are configured.

## Step 2 — Add public rendezvous observability and source accounting

Status: implemented; pending full validation.

Goal: make public rendezvous fallback obvious in snapshots, metrics, and peer-book/source reporting.

Scope:

- Add snapshot/status counts for public rendezvous candidates and decisions.
- Record public rendezvous candidates with a distinct source or clear fallback observability instead of merging them invisibly with operator rendezvous.
- Report whether bootstrap, rendezvous, or relay public fallback was used.
- Keep public rendezvous DNS resolution best-effort and disabled when `mode = "disabled"`.

Acceptance criteria:

- Public rendezvous endpoints can be supplied by app distributions without requiring manual user config.
- Private-infrastructure-only mode never reports public rendezvous use.
- Public fallback status shows whether bootstrap, rendezvous, or relay fallback was used.
- `NodeSnapshot` exposes category-level public fallback booleans, reasons, and rendezvous candidate count.
- Peer-book source accounting distinguishes public rendezvous from operator rendezvous.

## Step 3 — Auto-dial app-namespace DHT providers

Status: implemented; pending full validation.

Goal: when DHT provider discovery finds peers for the app namespace, automatically attempt network-layer connection when policy allows it.

Scope:

- On `GetProviders` results, record the peer namespace as today.
- If `auto_connect_discovered_peers` is true, build a connection plan for peer-book addresses and dial them.
- If no address is known yet, keep the peer in the peer book as discovered and dial later after Identify/Kademlia address updates arrive.
- Do not add the peer as a trusted contact.

Acceptance criteria:

- DHT-discovered app peers can transition from `discovered` to `connected` automatically.
- Auto-dial is bounded and deduplicated so repeated DHT provider results do not spam dials.
- Public/private policy can disable this behavior.

Implemented notes:

- `DhtProviderState` now tracks provider peers waiting for addresses and provider peers that already had one bounded auto-connect attempt.
- `GetProviders` results record the app namespace in the peer book and trigger policy-gated network-layer dialing when addresses are available.
- Kademlia address updates for already discovered DHT-provider peers are recorded into the peer book and can trigger the delayed auto-dial path.
- Auto-connect uses the connection strategy planner, so direct QUIC/TCP ordering and relay fallback policy stay centralized.
- This does not add trusted contacts; it only attempts transport-level connectivity.

## Step 4 — Auto-dial rendezvous-discovered app peers through connection strategy

Status: implemented; pending full validation.

Goal: move rendezvous-discovered peer dialing through the connection strategy planner instead of direct one-off dials.

Scope:

- Feed rendezvous peer-record addresses into the peer book.
- If auto-connect is enabled, create a connection plan using direct QUIC/TCP first and relay fallback after that.
- Keep the existing immediate address learning and cache recording behavior.
- Avoid duplicate dials for peers with an existing connection or pending plan.

Acceptance criteria:

- Rendezvous discovery produces direct/relay ordered connection attempts.
- Public relay candidates are usable as fallbacks when direct paths fail.
- Auto-connect still does not imply contact trust.

Implemented notes:

- Rendezvous `Discovered` events now record peer namespaces and addresses before dialing.
- Rendezvous-discovered peers are auto-dialed through the shared `auto_dial_peer_from_book(...)` helper.
- The old one-off `swarm.dial(addr.clone())` path inside rendezvous discovery was removed.
- Existing peer-cache recording and Kademlia address learning remain in place.
- Public rendezvous discoveries preserve `public_rendezvous` source accounting when the rendezvous node itself came from the public fallback set.
- Pending connection plans prevent duplicate dials for peers already connected or already in progress.
- No trusted contact state is modified by rendezvous auto-connect.

## Step 5 — Complete public relay fallback for NAT-to-NAT first launch

Status: implemented; pending full validation.

Goal: make public relay candidates usable by default app distributions that provide real relay/mediator endpoints.

Scope:

- Select public relay candidates when no operator/cached/rendezvous relays are available in `fallback_only` mode.
- Attempt relay reservation when the resolved profile supports relay client behavior.
- Keep DCUtR enabled so direct upgrade is attempted after relay connectivity exists.
- Preserve honest defaults: the shared repo still ships no fake relay fleet.

Acceptance criteria:

- When real public relays are configured, NATed nodes can reserve and connect through relay fallback.
- `Relay Discovery` status shows selected public relay candidates.
- DCUtR attempts are visible in the snapshot/pulse stream.

Implemented notes:

- Public fallback relay candidates flow through the same deterministic relay selector as configured, cached, and rendezvous candidates.
- Startup now reserves **selected** relays instead of treating the reservation path as configured-relays-only. Selected relays may come from operator config, cache, rendezvous, or public fallback.
- `ResolvedNodeConfig` now exposes `should_reserve_selected_relays` and `should_seed_selected_relays` so profile policy can cover public fallback relay candidates without overloading the older configured-relay fields.
- App distributions can provide real public relay/mediator DNSADDR entries in `discovery.public_bootstrap.relay_peers`; this shared repo still ships no fake public relay fleet.
- DCUtR remains enabled whenever relay-client capability is enabled, so relayed connectivity can later attempt direct upgrade while keeping relay fallback.

## Step 6 — Add consumer-vs-operator examples and advanced override docs

Status: planned.

Goal: make docs match the desired product behavior.

Scope:

- Add a consumer default walkthrough: run two apps, public fallback joins network, network peers connect, contacts remain untrusted until invite/QR/join-code.
- Keep private-infrastructure-first docs as advanced/operator mode.
- Document manual `bootstrap_peers`, `discovery.rendezvous_peers`, `relay_peers`, and `discovery.public_bootstrap.mode = "disabled"` overrides.
- Explain why public bootstrap alone is insufficient for NAT-to-NAT reliability without relay/rendezvous endpoints.

Acceptance criteria:

- README and operator docs no longer imply private infrastructure is the normal default.
- Public fallback docs distinguish bootstrap, rendezvous, relay, DHT provider discovery, auto-connect, and contact trust.

## Step 7 — Add runtime status for known vs discovered vs dialed vs connected

Status: planned.

Goal: prevent the UI confusion seen in the current output where known/discovered peers appear but connected peers remain zero.

Scope:

- Add snapshot counters for auto-connect attempts, pending connection plans, public rendezvous candidates, and discovered peers awaiting addresses.
- Add clearer pulse messages when a peer is only known/discovered but not yet dialable.
- Preserve current connected peer counts.

Acceptance criteria:

- The dashboard can explain why `PeerBook: known/discovered` is nonzero while `Connected Peers` is zero.
- Public fallback usage is visible per category.

## Step 8 — Add deterministic public-network resurrection tests

Status: planned.

Goal: add tests proving the default consumer public-network model works without requiring the CI runner to contact real public infrastructure.

Scope:

- Unit-test public fallback decisions for bootstrap, rendezvous, and relay candidates.
- Integration-test auto-connect planning for DHT and rendezvous discovered app peers with fake addresses.
- Test private-infrastructure-only mode disables public fallback and public auto-connect.
- Test that auto-connect never adds trusted contacts.

Acceptance criteria:

- CI validates the intended default UX deterministically.
- No test depends on live public bootstrap, rendezvous, or relay servers.
