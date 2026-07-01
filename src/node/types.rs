//! Node configuration, live snapshot, and internal control messages.

use std::collections::VecDeque;
use std::path::Path;

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

use crate::connectivity::discovery::DiscoveryConfig;
use crate::connectivity::dns::DnsaddrConfig;
use crate::connectivity::limits::ConnectionLimitsConfig;
use crate::connectivity::relay::{RelayServiceConfig, RelayServiceHealth, RelayState};
use crate::protocol::pulse::MessageSecurityConfig;

use super::profile::{NodeProfile, ResolvedNodeConfig};

/// Swarm + heartbeat configuration for a standalone P2P network instance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NodeConfig {
    /// High-level node profile. `auto` preserves current all-in-one behaviour until
    /// environment-aware resolution is implemented.
    #[serde(default)]
    pub profile: NodeProfile,
    pub network_id: u32,
    pub heartbeat_interval_secs: u64,
    pub startup_peer_cache_probe: usize,
    /// Stable libp2p node identity key file. Created on first run and reused after that.
    pub identity_key_path: String,
    /// Multiaddrs this node should listen on. Missing config uses safe all-in-one defaults.
    pub listen_addresses: Vec<String>,
    /// Public bootstrap peers to dial on startup. Full /p2p/<PeerId> multiaddrs are required.
    pub bootstrap_peers: Vec<String>,
    /// Discovery, peer-cache, bootstrap seed, and rendezvous controls.
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    /// `/dnsaddr` TXT lookup policy. Defaults to bounded DoH and can be pointed
    /// at an internal/self-hosted DoH resolver for production deployments.
    #[serde(default)]
    pub dnsaddr: DnsaddrConfig,
    /// Known Circuit Relay peers to dial on startup. Full /p2p/<PeerId> multiaddrs are required.
    pub relay_peers: Vec<String>,
    /// If true, each configured relay peer is also used for an outbound relay reservation.
    #[serde(default = "default_true")]
    pub reserve_configured_relays: bool,
    /// Global connection caps for the node. Enabled by default with conservative limits.
    #[serde(default)]
    pub connection_limits: ConnectionLimitsConfig,
    /// Message validation, replay, and peer scoring controls for heartbeat gossip.
    #[serde(default)]
    pub message_security: MessageSecurityConfig,
    /// Optional libp2p Circuit Relay v2 server mode. Disabled by default; relay client/DCUtR stay enabled.
    #[serde(default)]
    pub relay: RelayServiceConfig,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            profile: NodeProfile::Auto,
            network_id: 1,
            heartbeat_interval_secs: 30,
            startup_peer_cache_probe: 5,
            identity_key_path: ".p2p-net-identity-key".to_string(),
            listen_addresses: default_listen_addresses(),
            bootstrap_peers: Vec::new(),
            discovery: DiscoveryConfig::default(),
            dnsaddr: DnsaddrConfig::default(),
            relay_peers: Vec::new(),
            reserve_configured_relays: true,
            connection_limits: ConnectionLimitsConfig::default(),
            message_security: MessageSecurityConfig::default(),
            relay: RelayServiceConfig::default(),
        }
    }
}

impl NodeConfig {
    /// Load a JSON node config from disk. Missing fields use safe defaults,
    /// so a file with only `{ "relay": { "enabled": true } }` is valid.
    pub fn load_json_file(path: impl AsRef<Path>) -> Result<Self, crate::common::error::NetError> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|err| {
            crate::common::error::NetError::Config {
                path: path.display().to_string(),
                reason: err.to_string(),
            }
        })?;
        let cfg: Self =
            serde_json::from_str(&raw).map_err(|err| crate::common::error::NetError::Config {
                path: path.display().to_string(),
                reason: err.to_string(),
            })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Render the current config as pretty JSON for users who want an example config file.
    pub fn to_pretty_json(&self) -> Result<String, crate::common::error::NetError> {
        serde_json::to_string_pretty(self).map_err(|err| crate::common::error::NetError::Config {
            path: "<memory>".to_string(),
            reason: err.to_string(),
        })
    }

    /// Validate user-provided config before building a swarm. This rejects bad peer IDs,
    /// malformed listen addresses, and malformed bootstrap/relay addresses instead of
    /// silently ignoring them.
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if self.heartbeat_interval_secs == 0 {
            return Err(config_error("heartbeat_interval_secs must be at least 1"));
        }
        if self.identity_key_path.trim().is_empty() {
            return Err(config_error("identity_key_path must not be empty"));
        }

        self.dnsaddr.validate()?;
        validate_listen_addrs("listen_addresses", &self.listen_addresses)?;
        validate_peer_addrs("bootstrap_peers", &self.bootstrap_peers, true)?;
        validate_dnsaddr_use("bootstrap_peers", &self.bootstrap_peers, &self.dnsaddr)?;
        validate_peer_addrs(
            "discovery.bootstrap_seed_peers",
            &self.discovery.bootstrap_seed_peers,
            true,
        )?;
        validate_dnsaddr_use(
            "discovery.bootstrap_seed_peers",
            &self.discovery.bootstrap_seed_peers,
            &self.dnsaddr,
        )?;
        validate_peer_addrs(
            "discovery.rendezvous_peers",
            &self.discovery.rendezvous_peers,
            true,
        )?;
        validate_dnsaddr_use(
            "discovery.rendezvous_peers",
            &self.discovery.rendezvous_peers,
            &self.dnsaddr,
        )?;
        validate_peer_addrs("relay_peers", &self.relay_peers, true)?;
        validate_dnsaddr_use("relay_peers", &self.relay_peers, &self.dnsaddr)?;
        self.discovery.validate()?;
        self.connection_limits.validate()?;
        self.message_security.validate()?;
        self.relay.validate()?;
        Ok(())
    }

    /// Return a clone with phase-1 profile defaults applied. Explicit profiles are
    /// intentionally centralized here so start-up code does not scatter role rules.
    pub fn with_profile_defaults_applied(&self) -> Self {
        let mut cfg = self.clone();
        cfg.profile.apply_to(&mut cfg);
        cfg
    }

    /// Resolve the high-level profile into a concrete role and behaviour set.
    pub fn resolved(&self) -> ResolvedNodeConfig {
        ResolvedNodeConfig::from_config(self)
    }

    pub fn parsed_listen_addresses(
        &self,
    ) -> Result<Vec<Multiaddr>, crate::common::error::NetError> {
        parse_multiaddrs("listen_addresses", &self.listen_addresses)
    }

    pub fn parsed_bootstrap_peers(&self) -> Result<Vec<Multiaddr>, crate::common::error::NetError> {
        parse_multiaddrs("bootstrap_peers", &self.bootstrap_peers)
    }

    pub fn parsed_relay_peers(&self) -> Result<Vec<Multiaddr>, crate::common::error::NetError> {
        parse_multiaddrs("relay_peers", &self.relay_peers)
    }

    pub fn parsed_bootstrap_seed_peers(
        &self,
    ) -> Result<Vec<Multiaddr>, crate::common::error::NetError> {
        parse_multiaddrs(
            "discovery.bootstrap_seed_peers",
            &self.discovery.bootstrap_seed_peers,
        )
    }

    pub fn parsed_rendezvous_peers(
        &self,
    ) -> Result<Vec<Multiaddr>, crate::common::error::NetError> {
        parse_multiaddrs(
            "discovery.rendezvous_peers",
            &self.discovery.rendezvous_peers,
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeSnapshot {
    pub network_id: u32,
    pub network_label: String,
    pub peer_id: String,
    pub nat_status: String,
    pub public_addr: Option<String>,
    pub active_transports: Vec<String>,
    pub connected_peers: usize,
    pub relay_server_enabled: bool,
    pub relay_service_health: RelayServiceHealth,
    /// ACL scope is intentionally connection-level for now because rust-libp2p's stock relay server
    /// does not expose a relay-only reservation hook.
    pub relay_acl_scope: String,
    /// Active reservations accepted by this node while acting as a relay server.
    pub relay_reservations_accepted: usize,
    /// Active circuits currently served by this node as a relay server.
    /// rust-libp2p relay-client events do not expose a matching close event, so client-side
    /// circuit establishment pulses are logged but not counted as active here.
    pub relay_active_circuits: usize,
    /// Total relay requests denied by this relay server, across reservations and circuits.
    pub relay_denied_requests: usize,
    /// Bytes forwarded by this relay server when byte accounting is available.
    /// rust-libp2p relay events currently do not expose per-circuit byte totals, so this remains zero unless a future accounting layer updates it.
    pub relay_bytes_forwarded: u64,
    pub relay_denied_reservations: usize,
    pub relay_denied_circuits: usize,
    pub relay_rate_limited_events: usize,
    pub relay_at_capacity_events: usize,
    pub relay_server_errors: usize,
    pub connection_limit_events: usize,
    pub connection_cap_disconnects: usize,
    /// Relay reservations this node has established as a relay client.
    pub relay_client_reservations: usize,
    /// Relay reservation attempts this node has initiated as a relay client.
    pub relay_client_reservation_attempts: usize,
    /// Relay reservation failures observed locally while setting up configured relays.
    pub relay_client_reservation_failures: usize,
    /// Confirmed `/p2p-circuit` listen addresses for this node.
    pub relayed_listen_addresses: Vec<String>,
    /// DCUtR upgrade attempts/events observed.
    pub dcutr_attempts: usize,
    /// DCUtR events that look successful by event debug output.
    pub dcutr_successes: usize,
    pub rendezvous_client_enabled: bool,
    pub rendezvous_server_enabled: bool,
    pub rendezvous_registered_with: usize,
    pub rendezvous_discovered_peers: usize,
    pub rendezvous_register_attempts: usize,
    pub rendezvous_register_failures: usize,
    pub rendezvous_discover_attempts: usize,
    pub rendezvous_discover_failures: usize,
    pub rendezvous_server_registrations: usize,
    pub rendezvous_server_discoveries_served: usize,
    pub rendezvous_server_errors: usize,
    pub gossip_messages_rejected: usize,
    pub gossip_messages_ignored: usize,
    pub gossip_messages_accepted: usize,
    pub pulses: VecDeque<String>,
    pub uptime_secs: u64,
}

impl NodeSnapshot {
    /// Apply relay/DCUtR counters from the authoritative event-loop relay state.
    pub fn apply_relay_state(&mut self, relay_state: &RelayState) {
        self.relay_server_enabled = relay_state.server_enabled;
        self.relay_service_health = relay_state.health;
        self.relay_reservations_accepted = relay_state.accepted_reservations;
        self.relay_client_reservations = relay_state.relay_client_reservations.len();
        self.relay_client_reservation_attempts = relay_state.relay_client_reservation_attempts;
        self.relay_client_reservation_failures = relay_state.relay_client_reservation_failures;
        self.relay_active_circuits = relay_state.active_circuits;
        self.relay_denied_reservations = relay_state.denied_reservations;
        self.relay_denied_circuits = relay_state.denied_circuits;
        self.relay_denied_requests = relay_state
            .denied_reservations
            .saturating_add(relay_state.denied_circuits);
        self.relay_rate_limited_events = relay_state.rate_limited_events;
        self.relay_at_capacity_events = relay_state.at_capacity_events;
        self.relay_server_errors = relay_state.server_errors;
        self.relay_bytes_forwarded = relay_state.relay_bytes_forwarded;
        self.relayed_listen_addresses = relay_state.relayed_listen_addrs.iter().cloned().collect();
        self.dcutr_attempts = relay_state.dcutr_attempts;
        self.dcutr_successes = relay_state.dcutr_successes;
    }
}

pub(crate) fn network_label(network_id: u32) -> String {
    if network_id == 0 {
        "MAINNET".to_string()
    } else {
        format!("TESTNET-{network_id}")
    }
}

pub(crate) fn default_listen_addresses() -> Vec<String> {
    vec![
        "/ip4/0.0.0.0/udp/4001/quic-v1".to_string(),
        "/ip4/0.0.0.0/tcp/4001".to_string(),
        "/ip4/0.0.0.0/tcp/4002/ws".to_string(),
    ]
}

fn default_true() -> bool {
    true
}

fn parse_multiaddrs(
    field: &str,
    values: &[String],
) -> Result<Vec<Multiaddr>, crate::common::error::NetError> {
    values
        .iter()
        .map(|raw| {
            raw.parse::<Multiaddr>().map_err(|err| {
                config_error(format!("{field} contains invalid multiaddr `{raw}`: {err}"))
            })
        })
        .collect()
}

fn validate_listen_addrs(
    field: &str,
    values: &[String],
) -> Result<(), crate::common::error::NetError> {
    for raw in values {
        let addr = raw.parse::<Multiaddr>().map_err(|err| {
            config_error(format!("{field} contains invalid multiaddr `{raw}`: {err}"))
        })?;
        if contains_dns_protocol(&addr) {
            return Err(config_error(format!(
                "{field} entries must use concrete /ip4 or /ip6 listen addresses; DNS multiaddrs are only supported for dialing: `{raw}`"
            )));
        }
    }
    Ok(())
}

fn validate_peer_addrs(
    field: &str,
    values: &[String],
    require_p2p: bool,
) -> Result<(), crate::common::error::NetError> {
    for raw in values {
        let _addr = raw.parse::<Multiaddr>().map_err(|err| {
            config_error(format!("{field} contains invalid multiaddr `{raw}`: {err}"))
        })?;
        if let Some(peer_id) = extract_p2p_peer_id(raw) {
            peer_id.parse::<PeerId>().map_err(|err| {
                config_error(format!(
                    "{field} contains invalid peer id `{peer_id}` in `{raw}`: {err}"
                ))
            })?;
        } else if require_p2p {
            return Err(config_error(format!(
                "{field} entries must include /p2p/<PeerId>; bad entry `{raw}`"
            )));
        }
    }
    Ok(())
}

fn contains_dns_protocol(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| {
        matches!(
            protocol,
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_)
        )
    })
}

fn validate_dnsaddr_use(
    field: &str,
    values: &[String],
    dnsaddr: &DnsaddrConfig,
) -> Result<(), crate::common::error::NetError> {
    if dnsaddr.enabled {
        return Ok(());
    }
    for raw in values {
        let addr = raw.parse::<Multiaddr>().map_err(|err| {
            config_error(format!("{field} contains invalid multiaddr `{raw}`: {err}"))
        })?;
        if addr
            .iter()
            .any(|protocol| matches!(protocol, Protocol::Dnsaddr(_)))
        {
            return Err(config_error(format!(
                "{field} contains /dnsaddr entry `{raw}` but dnsaddr.enabled is false"
            )));
        }
    }
    Ok(())
}

fn extract_p2p_peer_id(raw: &str) -> Option<&str> {
    let (_, tail) = raw.split_once("/p2p/")?;
    Some(tail.split('/').next().unwrap_or(tail))
}

fn config_error(reason: impl Into<String>) -> crate::common::error::NetError {
    crate::common::error::NetError::Config {
        path: "<config>".to_string(),
        reason: reason.into(),
    }
}
