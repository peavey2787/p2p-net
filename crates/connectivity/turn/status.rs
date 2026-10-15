//! TURN status exported in node snapshots.

use serde::Serialize;

/// Built-in TURN server lifecycle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnServerState {
    #[default]
    Disabled,
    /// Enabled; waiting for `public_ip_probe` to find the relay address.
    WaitingForPublicIp,
    Running,
    Failed,
}

impl TurnServerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::WaitingForPublicIp => "waiting_for_public_ip",
            Self::Running => "running",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TurnSnapshot {
    /// ICE servers currently handed to new browser peer connections.
    pub ice_servers: usize,
    pub turn_servers: usize,
    /// `browser_webrtc` ICE policy in force: `all` or `relay`.
    pub ice_transport_policy: String,
    pub stun_enabled: bool,
    /// Active `turn.client` sources in preference order.
    pub client_sources: Vec<String>,
    /// Broker discovery trust: `off` (no `libp2p` source), `same_host` or `any`.
    pub discovery: String,
    pub client_endpoints_active: usize,
    pub client_endpoint_errors: usize,
    pub broker_peers: usize,
    pub credential_requests_sent: usize,
    pub credential_servers_accepted: usize,
    pub credential_servers_rejected: usize,
    pub server_state: TurnServerState,
    pub server_listen: Option<String>,
    pub server_urls: Vec<String>,
    pub server_allocations: usize,
    pub server_allocation_capacity: usize,
    pub server_error: Option<String>,
    pub broker_enabled: bool,
    pub broker_sources: Vec<String>,
    pub broker_ready_sources: usize,
    pub broker_granted: usize,
    pub broker_rate_limited: usize,
    pub broker_unavailable: usize,
    pub broker_source_errors: usize,
    pub broker_last_error: Option<String>,
}
