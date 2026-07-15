//! Node configuration and config file helpers.

use std::path::Path;

use crate::common::error::NetError;
use crate::connectivity::dcutr::DcutrPolicy;
use crate::connectivity::discovery::DiscoveryConfig;
use crate::connectivity::dns::DnsaddrConfig;
use crate::connectivity::limits::ConnectionLimitsConfig;
use crate::connectivity::mediator::MediatorConfig;
use crate::connectivity::relay::RelayServiceConfig;
use crate::connectivity::webrtc::DEFAULT_WEBRTC_DIRECT_LISTEN_ADDR;
use crate::platform::PlatformRuntime;
use crate::protocol::pulse::MessageSecurityConfig;

use libp2p::Multiaddr;

use super::config_validation::{parse_multiaddrs, validate_node_config};
use super::environment::{EnvironmentConfig, EnvironmentReport};
use super::profile::{NodeProfile, ResolvedNodeConfig};
use super::public_ip::PublicIpProbeConfig;

/// Swarm + heartbeat configuration for a standalone P2P network instance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NodeConfig {
    /// High-level node profile. `auto` can be resolved against an advisory environment report.
    #[serde(default)]
    pub profile: NodeProfile,
    /// Optional platform/network hints for advisory environment detection.
    #[serde(default)]
    pub environment: EnvironmentConfig,
    pub network_id: u32,
    pub heartbeat_interval_secs: u64,
    pub startup_peer_cache_probe: usize,
    /// Stable libp2p node identity key file. Created on first run and reused after that.
    pub identity_key_path: String,
    /// Multiaddrs this node should listen on. Missing config uses safe shared-node defaults.
    pub listen_addresses: Vec<String>,
    /// Public bootstrap peers to dial on startup. Full /p2p/<PeerId> multiaddrs are required.
    pub bootstrap_peers: Vec<String>,
    /// Discovery, peer-cache, bootstrap seed, and rendezvous controls.
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    /// Optional HTTPS public-IP probe used to advertise concrete external addresses.
    #[serde(default)]
    pub public_ip_probe: PublicIpProbeConfig,
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
    /// Direct Connection Upgrade through Relay policy. Enabled by default with relay fallback retained.
    #[serde(default)]
    pub dcutr: DcutrPolicy,
    /// First-class DCUtR mediator policy. Disabled by default; when enabled it maps
    /// to the Circuit Relay v2 server capability through the central resolver.
    #[serde(default)]
    pub mediator: MediatorConfig,
    /// Optional libp2p Circuit Relay v2 server mode. Disabled by default; relay client/DCUtR stay enabled.
    #[serde(default)]
    pub relay: RelayServiceConfig,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            profile: NodeProfile::Auto,
            environment: EnvironmentConfig::default(),
            network_id: 1,
            heartbeat_interval_secs: 30,
            startup_peer_cache_probe: 5,
            identity_key_path: ".p2p-net-identity-key".to_string(),
            listen_addresses: default_listen_addresses(),
            bootstrap_peers: Vec::new(),
            discovery: DiscoveryConfig::default(),
            public_ip_probe: PublicIpProbeConfig::default(),
            dnsaddr: DnsaddrConfig::default(),
            relay_peers: Vec::new(),
            reserve_configured_relays: true,
            connection_limits: ConnectionLimitsConfig::default(),
            message_security: MessageSecurityConfig::default(),
            dcutr: DcutrPolicy::default(),
            mediator: MediatorConfig::default(),
            relay: RelayServiceConfig::default(),
        }
    }
}

impl NodeConfig {
    /// Load a JSON node config from disk. Missing fields use safe defaults,
    /// so a file with only `{ "relay": { "enabled": true } }` is valid.
    pub fn load_json_file(path: impl AsRef<Path>) -> Result<Self, NetError> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|err| NetError::Config {
            path: path.display().to_string(),
            reason: err.to_string(),
        })?;
        let cfg: Self = serde_json::from_str(&raw).map_err(|err| NetError::Config {
            path: path.display().to_string(),
            reason: err.to_string(),
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Render the current config as pretty JSON for users who want an example config file.
    pub fn to_pretty_json(&self) -> Result<String, NetError> {
        serde_json::to_string_pretty(self).map_err(|err| NetError::Config {
            path: "<memory>".to_string(),
            reason: err.to_string(),
        })
    }

    /// Validate user-provided config before building a swarm. This rejects bad peer IDs,
    /// malformed listen addresses, and malformed bootstrap/relay addresses instead of
    /// silently ignoring them.
    pub fn validate(&self) -> Result<(), NetError> {
        validate_node_config(self)
    }

    /// Return a clone with explicit profile defaults applied. Profile defaults
    /// are centralized here so startup code does not scatter role rules.
    pub fn with_profile_defaults_applied(&self) -> Self {
        let mut cfg = self.clone();
        cfg.profile.apply_to(&mut cfg);
        cfg
    }

    /// Apply a resolved capability policy to a clone of this config. The
    /// returned config is the effective runtime view used by startup.
    pub fn with_resolved_capabilities_applied(&self, resolved: &ResolvedNodeConfig) -> Self {
        super::capabilities::apply_resolved_capabilities(self, resolved)
    }

    /// Resolve the high-level profile into a concrete role and behaviour set.
    /// Callers that do not provide environment information still get a
    /// deterministic desktop-friendly resolution.
    pub fn resolved(&self) -> ResolvedNodeConfig {
        ResolvedNodeConfig::from_config(self)
    }

    /// Fallible form of `resolved` that reports impossible capability
    /// combinations instead of panicking.
    pub fn try_resolved(&self) -> Result<ResolvedNodeConfig, NetError> {
        ResolvedNodeConfig::try_from_config(self)
    }

    /// Resolve `profile = auto` using an advisory environment report. Explicit
    /// profiles still win over environment detection.
    pub fn resolved_for_environment(&self, environment: &EnvironmentReport) -> ResolvedNodeConfig {
        ResolvedNodeConfig::from_config_and_environment(self, environment)
    }

    /// Fallible form of `resolved_for_environment` that reports impossible
    /// capability combinations before startup.
    pub fn try_resolved_for_environment(
        &self,
        environment: &EnvironmentReport,
    ) -> Result<ResolvedNodeConfig, NetError> {
        ResolvedNodeConfig::try_from_config_and_environment(self, environment)
    }

    /// Build the current advisory environment report from config/platform hints.
    pub fn environment_report(&self) -> EnvironmentReport {
        EnvironmentReport::detect(self)
    }

    /// Build the advisory environment report with facts supplied by an embedding
    /// platform runtime. Explicit config hints still override runtime hints.
    pub fn environment_report_with_runtime(
        &self,
        runtime: &dyn PlatformRuntime,
    ) -> EnvironmentReport {
        EnvironmentReport::detect_with_runtime(self, runtime)
    }

    pub fn parsed_listen_addresses(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs("listen_addresses", &self.listen_addresses)
    }

    pub fn parsed_bootstrap_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs("bootstrap_peers", &self.bootstrap_peers)
    }

    pub fn parsed_relay_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs("relay_peers", &self.relay_peers)
    }

    pub fn parsed_bootstrap_seed_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.bootstrap_seed_peers",
            &self.discovery.bootstrap_seed_peers,
        )
    }

    pub fn parsed_public_bootstrap_seed_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.public_bootstrap.bootstrap_seed_peers",
            &self.discovery.public_bootstrap.bootstrap_seed_peers,
        )
    }

    pub fn parsed_public_rendezvous_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.public_bootstrap.rendezvous_peers",
            &self.discovery.public_bootstrap.rendezvous_peers,
        )
    }

    pub fn parsed_public_relay_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.public_bootstrap.relay_peers",
            &self.discovery.public_bootstrap.relay_peers,
        )
    }

    pub fn parsed_rendezvous_peers(&self) -> Result<Vec<Multiaddr>, NetError> {
        parse_multiaddrs(
            "discovery.rendezvous_peers",
            &self.discovery.rendezvous_peers,
        )
    }
}

fn default_listen_addresses() -> Vec<String> {
    vec![
        "/ip4/0.0.0.0/udp/4001/quic-v1".to_string(),
        DEFAULT_WEBRTC_DIRECT_LISTEN_ADDR.to_string(),
        "/ip4/0.0.0.0/tcp/4001".to_string(),
        "/ip4/0.0.0.0/tcp/4002/ws".to_string(),
    ]
}

fn default_true() -> bool {
    true
}
