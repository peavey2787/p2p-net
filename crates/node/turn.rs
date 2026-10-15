//! Runtime TURN state: the browser TURN server set (composed into WebRTC's
//! ICE configuration by `browser_webrtc`), the credential broker, and
//! (natively) the built-in TURN server. Swarm event glue lives in
//! `events::turn`; policy and protocol logic live in `connectivity::turn`.

use libp2p::{Multiaddr, PeerId};
use libp2p_webrtc_websys::IceServers;
use web_time::Instant;

use crate::connectivity::ice::IceTransportPolicy;
use crate::connectivity::turn::{
    CredentialRequest, CredentialResponse, DiscoveryTrust, ProviderRefresher, TurnBrokerState,
    TurnClientConfig, TurnClientState, TurnProviderConfig, TurnServerState, TurnSnapshot,
    TurnSource,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::connectivity::turn::{RunningTurnServer, TurnServerConfig};

use super::config::{BrowserIcePlan, NodeConfig};
use super::profile::ResolvedNodeConfig;

pub(crate) struct TurnRuntime {
    client_cfg: TurnClientConfig,
    ice_plan: BrowserIcePlan,
    network_client_enabled: bool,
    client: Option<TurnClient>,
    broker: Option<TurnBrokerState>,
    #[cfg(not(target_arch = "wasm32"))]
    server: TurnServerSlot,
    snapshot: TurnSnapshot,
}

/// Browser side: TURN servers composed with the WebRTC ICE plan and pushed
/// into the signaling handle.
struct TurnClient {
    state: TurnClientState,
    endpoints: Vec<(TurnSource, ProviderRefresher)>,
    ice_servers: IceServers,
    network_enabled: bool,
}

#[cfg(not(target_arch = "wasm32"))]
struct TurnServerSlot {
    cfg: Option<TurnServerConfig>,
    running: Option<RunningTurnServer>,
}

impl TurnRuntime {
    pub(crate) fn new(cfg: &NodeConfig, resolved: &ResolvedNodeConfig) -> Self {
        let turn = &cfg.turn;
        let client_cfg = cfg.browser_webrtc.with_legacy_turn(&turn.client);
        let ice_plan = cfg.browser_webrtc.plan();
        let broker = resolved
            .turn_broker_enabled
            .then(|| TurnBrokerState::new(&turn.broker));
        let snapshot = TurnSnapshot {
            ice_transport_policy: wire_name(&ice_plan.policy),
            stun_enabled: ice_plan.stun_enabled && ice_plan.policy == IceTransportPolicy::All,
            client_sources: client_cfg
                .sources()
                .iter()
                .map(|source| source.as_str().to_string())
                .collect(),
            discovery: client_cfg.discovery().map_or_else(
                || "off".to_string(),
                |trust: DiscoveryTrust| wire_name(&trust),
            ),
            broker_enabled: broker.is_some(),
            server_state: if resolved.turn_server_enabled {
                TurnServerState::WaitingForPublicIp
            } else {
                TurnServerState::Disabled
            },
            server_allocation_capacity: turn.server.allocation_capacity(),
            ..TurnSnapshot::default()
        };
        let mut runtime = Self {
            client_cfg,
            ice_plan,
            network_client_enabled: resolved.turn_network_client_enabled,
            client: None,
            broker,
            #[cfg(not(target_arch = "wasm32"))]
            server: TurnServerSlot {
                cfg: resolved.turn_server_enabled.then(|| turn.server.clone()),
                running: None,
            },
            snapshot,
        };
        runtime.refresh_snapshot();
        runtime
    }

    /// Browser nodes: manage the WebRTC signaling ICE server list.
    pub(crate) fn attach_browser_ice_servers(&mut self, ice_servers: IceServers) {
        let cfg = &self.client_cfg;
        let endpoints = cfg
            .sources()
            .into_iter()
            .filter_map(|source| {
                let endpoint = cfg.endpoint(source)?.clone();
                let provider = TurnProviderConfig::Http(endpoint);
                Some((
                    source,
                    ProviderRefresher::new(provider, cfg.credential_ttl_secs),
                ))
            })
            .collect();
        self.client = Some(TurnClient {
            state: TurnClientState::new(cfg),
            endpoints,
            ice_servers,
            network_enabled: self.network_client_enabled,
        });
        self.publish_ice_servers();
    }

    pub(crate) fn on_connection(&mut self, peer: PeerId, remote: &Multiaddr) {
        if let Some(client) = &mut self.client {
            client.state.note_connection(peer, remote);
        }
    }

    pub(crate) fn on_disconnected(&mut self, peer: &PeerId) {
        if let Some(client) = &mut self.client {
            client.state.forget_peer(peer);
        }
    }

    /// Credential request for a peer that advertises the broker protocol.
    pub(crate) fn request_for(&mut self, peer: PeerId, now: Instant) -> Option<CredentialRequest> {
        let client = self
            .client
            .as_mut()
            .filter(|client| client.network_enabled)?;
        let request = client
            .state
            .should_request(peer, now)
            .then(|| CredentialRequest {
                ttl_secs: client.state.requested_ttl_secs(),
            })?;
        self.snapshot.credential_requests_sent += 1;
        Some(request)
    }

    pub(crate) fn mark_unsupported(&mut self, peer: PeerId) {
        if let Some(client) = &mut self.client {
            client.state.mark_unsupported(peer);
        }
    }

    /// Apply a broker's answer; returns a pulse line.
    pub(crate) fn on_response(
        &mut self,
        peer: PeerId,
        response: CredentialResponse,
        now: Instant,
    ) -> Option<String> {
        let client = self.client.as_mut()?;
        let line = match response {
            CredentialResponse::Granted(credentials) => {
                let offer = client.state.accept_from_broker(peer, credentials, now);
                self.snapshot.credential_servers_accepted += offer.accepted;
                self.snapshot.credential_servers_rejected += offer.rejected;
                format!(
                    "TURN_CREDENTIALS_RECEIVED peer={peer} accepted={} rejected={}",
                    offer.accepted, offer.rejected
                )
            }
            CredentialResponse::Unavailable => format!("TURN_CREDENTIALS_UNAVAILABLE peer={peer}"),
            CredentialResponse::RateLimited => format!("TURN_CREDENTIALS_RATE_LIMITED peer={peer}"),
        };
        self.publish_ice_servers();
        Some(line)
    }

    /// Answer a peer's credential request (broker nodes only).
    pub(crate) fn answer(&mut self, peer: PeerId, now: Instant) -> CredentialResponse {
        let response = match &mut self.broker {
            Some(broker) => {
                broker.maintain(now);
                broker.answer(peer, now)
            }
            None => CredentialResponse::Unavailable,
        };
        self.refresh_snapshot();
        response
    }

    /// Periodic work: refresh providers, prune expired credentials. Returns
    /// brokers to ask again and pulse lines.
    pub(crate) fn maintain(&mut self, now: Instant) -> (Vec<PeerId>, Vec<String>) {
        let mut pulses = Vec::new();
        let mut due = Vec::new();
        if let Some(broker) = &mut self.broker {
            pulses.extend(broker.maintain(now));
        }
        if let Some(client) = &mut self.client {
            pulses.extend(maintain_client_endpoints(client, &mut self.snapshot, now));
            client.state.prune(now);
            if client.network_enabled {
                due = client.state.brokers_due(now);
            }
        }
        self.publish_ice_servers();
        (due, pulses)
    }

    pub(crate) fn snapshot(&self) -> &TurnSnapshot {
        &self.snapshot
    }

    fn publish_ice_servers(&mut self) {
        if let Some(client) = &self.client {
            let composed = self.ice_plan.compose(&client.state.effective());
            self.snapshot.ice_servers = composed.len();
            self.snapshot.turn_servers = composed.iter().filter(|server| server.is_turn()).count();
            client
                .ice_servers
                .replace(composed.iter().map(crate::IceServer::to_webrtc).collect());
        }
        self.refresh_snapshot();
    }

    fn refresh_snapshot(&mut self) {
        if let Some(client) = &self.client {
            let status = client.state.status();
            self.snapshot.client_endpoints_active = status.endpoints_active;
            self.snapshot.broker_peers = status.broker_peers;
        }
        if let Some(broker) = &self.broker {
            let status = broker.status();
            self.snapshot.broker_sources = status.sources.iter().map(|s| s.to_string()).collect();
            self.snapshot.broker_ready_sources = status.ready_sources;
            self.snapshot.broker_granted = status.granted;
            self.snapshot.broker_rate_limited = status.rate_limited;
            self.snapshot.broker_unavailable = status.unavailable;
            self.snapshot.broker_source_errors = status.source_errors;
            self.snapshot.broker_last_error = status.last_error.clone();
        }
    }
}

/// The serde (snake_case) name of a unit enum value.
fn wire_name(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn maintain_client_endpoints(
    client: &mut TurnClient,
    snapshot: &mut TurnSnapshot,
    now: Instant,
) -> Vec<String> {
    let mut pulses = Vec::new();
    for (source, refresher) in &mut client.endpoints {
        let result = refresher.take_result(now);
        refresher.start_if_due(now);
        let source_name = source.as_str();
        match result {
            Some(Ok(credentials)) => {
                let servers = credentials.ice_servers.len();
                client.state.accept_from_endpoint(*source, credentials, now);
                pulses.push(format!(
                    "TURN_PROVIDER_REFRESHED source={source_name} servers={servers}"
                ));
            }
            Some(Err(error)) => {
                snapshot.client_endpoint_errors += 1;
                pulses.push(format!(
                    "TURN_PROVIDER_FAILED source={source_name} error={error}"
                ));
            }
            None => {}
        }
    }
    pulses
}

#[cfg(not(target_arch = "wasm32"))]
impl TurnRuntime {
    /// The configured relay address, if the operator set one.
    pub(crate) fn configured_relay_ip(&self) -> Option<std::net::IpAddr> {
        let cfg = self.server.cfg.as_ref()?;
        cfg.public_ip.as_deref()?.parse().ok()
    }

    /// Start the built-in TURN server once its relay address is known.
    /// Returns a pulse line; does nothing when disabled or already running.
    pub(crate) async fn start_server(&mut self, relay_ip: std::net::IpAddr) -> Option<String> {
        if self.server.running.is_some() {
            return None;
        }
        let cfg = self.server.cfg.as_ref()?;
        match RunningTurnServer::start(cfg, relay_ip).await {
            Ok(server) => {
                let provider = server.credential_provider();
                self.snapshot.server_state = TurnServerState::Running;
                self.snapshot.server_listen = Some(server.listen_addr().to_string());
                self.snapshot.server_urls = provider.urls.clone();
                self.snapshot.server_error = None;
                if let Some(broker) = &mut self.broker {
                    broker.add_source(TurnProviderConfig::SharedSecret(provider));
                    broker.maintain(Instant::now());
                }
                self.server.running = Some(server);
                self.refresh_snapshot();
                Some(format!(
                    "TURN_SERVER_STARTED listen={} relay_ip={relay_ip}",
                    self.snapshot.server_listen.as_deref().unwrap_or("?")
                ))
            }
            Err(error) => {
                self.snapshot.server_state = TurnServerState::Failed;
                self.snapshot.server_error = Some(error.clone());
                Some(format!("TURN_SERVER_FAILED error={error}"))
            }
        }
    }

    /// The public-IP probe finished without an address: the server cannot
    /// tell clients where to relay, so report why it is not running.
    pub(crate) fn public_ip_unavailable(&mut self) -> Option<String> {
        if self.server.cfg.is_none() || self.server.running.is_some() {
            return None;
        }
        let error = "no public IP found; set turn.server.public_ip".to_string();
        self.snapshot.server_state = TurnServerState::Failed;
        self.snapshot.server_error = Some(error.clone());
        Some(format!("TURN_SERVER_FAILED error={error}"))
    }

    pub(crate) async fn refresh_server_stats(&mut self) {
        if let Some(server) = &self.server.running {
            self.snapshot.server_allocations = server.allocation_count().await;
        }
    }

    pub(crate) async fn shutdown(&mut self) {
        if let Some(server) = self.server.running.take() {
            server.close().await;
        }
    }
}
