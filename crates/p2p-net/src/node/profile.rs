//! Profile and capability resolution for the standalone node.
//!
//! User-facing profiles and resolved capability policy for the standalone node.
//!
//! Callers choose a high-level `NodeProfile`; the central resolver turns it into
//! a concrete `NodeRole` and a libp2p-shaped `BehaviourSet`.

use serde::{Deserialize, Serialize};

use super::types::NodeConfig;

/// User-facing node profile. `Auto` delegates role selection to the central
/// capability resolver using advisory environment facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeProfile {
    /// Resolve to a concrete role from explicit config and environment facts.
    #[default]
    Auto,
    /// Reachable node that listens for inbound peers and participates directly.
    Full,
    /// Outbound-first node intended for NAT/CGNAT environments.
    Lite,
    /// Full node with Circuit Relay service enabled for generic relayed traffic.
    Relay,
    /// Full node intentionally advertising itself as a DCUtR mediator for lite/mobile peers.
    Mediator,
    /// Infrastructure node that enables rendezvous server capabilities.
    Rendezvous,
    /// Infrastructure node optimized for stable discovery/bootstrap addressing.
    Bootstrap,
    /// Lite profile with conservative assumptions for mobile/tablet platforms.
    MobileLite,
}

impl NodeProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Lite => "lite",
            Self::Relay => "relay",
            Self::Mediator => "mediator",
            Self::Rendezvous => "rendezvous",
            Self::Bootstrap => "bootstrap",
            Self::MobileLite => "mobile_lite",
        }
    }

    /// Apply explicit profile defaults to a clone of the raw config before
    /// capability resolution.
    pub(crate) fn apply_to(self, cfg: &mut NodeConfig) {
        match self {
            Self::Auto | Self::Full | Self::Bootstrap => {}
            Self::Lite | Self::MobileLite => {
                cfg.relay.enabled = false;
                cfg.discovery.rendezvous.server_enabled = false;
            }
            Self::Relay => {
                cfg.relay.enabled = true;
            }
            Self::Mediator => {
                cfg.mediator.enabled = true;
                cfg.relay.enabled = true;
            }
            Self::Rendezvous => {
                cfg.discovery.rendezvous.client_enabled = true;
                cfg.discovery.rendezvous.server_enabled = true;
            }
        }
    }
}

/// Concrete resolved role after profile defaults are applied. Unlike
/// `NodeProfile`, this never contains `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    Full,
    Lite,
    Relay,
    Mediator,
    Rendezvous,
    Bootstrap,
    MobileLite,
}

impl NodeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Lite => "lite",
            Self::Relay => "relay",
            Self::Mediator => "mediator",
            Self::Rendezvous => "rendezvous",
            Self::Bootstrap => "bootstrap",
            Self::MobileLite => "mobile_lite",
        }
    }
}

/// Profile-level feature map. This is intentionally libp2p-shaped but does not
/// expose libp2p types, keeping app policy separate from transport assembly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviourSet {
    pub gossipsub: bool,
    pub kademlia_client: bool,
    pub kademlia_server: bool,
    pub autonat: bool,
    pub dcutr: bool,
    pub relay_client: bool,
    pub relay_server: bool,
    pub rendezvous_client: bool,
    pub rendezvous_server: bool,
    pub identify: bool,
    pub ping: bool,
}

impl BehaviourSet {
    pub fn for_role(role: NodeRole, cfg: &NodeConfig) -> Self {
        let infrastructure = matches!(
            role,
            NodeRole::Relay | NodeRole::Mediator | NodeRole::Rendezvous | NodeRole::Bootstrap
        );
        let lite = matches!(role, NodeRole::Lite | NodeRole::MobileLite);

        Self {
            gossipsub: true,
            kademlia_client: true,
            kademlia_server: !lite,
            autonat: true,
            dcutr: true,
            relay_client: true,
            relay_server: matches!(role, NodeRole::Relay | NodeRole::Mediator)
                || cfg.relay.enabled
                || cfg.mediator.enabled,
            rendezvous_client: cfg.discovery.rendezvous.client_enabled || lite,
            rendezvous_server: matches!(role, NodeRole::Rendezvous)
                || cfg.discovery.rendezvous.server_enabled,
            identify: true,
            ping: true,
        }
        .with_infrastructure_defaults(infrastructure)
    }

    fn with_infrastructure_defaults(mut self, infrastructure: bool) -> Self {
        if infrastructure {
            self.kademlia_server = true;
            self.identify = true;
            self.ping = true;
        }
        self
    }
}

/// Resolved view of user config. Runtime code consumes this single policy
/// object instead of re-deciding what each profile means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedNodeConfig {
    pub profile: NodeProfile,
    pub role: NodeRole,
    pub enabled_behaviours: BehaviourSet,
    pub reserve_configured_relays: bool,
    pub should_reserve_configured_relays: bool,
    pub should_seed_relay_peers: bool,
    pub should_listen: bool,
    pub listen_addresses: Vec<String>,
    pub relay_peers: Vec<String>,
    pub relay_discovery_enabled: bool,
    pub relay_discovery_min_reservations: usize,
    pub relay_discovery_max_reservations: usize,
    pub dcutr_enabled: bool,
    pub dcutr_attempt_after_relay_connection: bool,
    pub dcutr_keep_relay_fallback: bool,
    pub dcutr_retry_interval_secs: u64,
    pub dcutr_max_attempts_per_peer: u32,
    pub relay_server_enabled: bool,
    pub mediator_enabled: bool,
    pub mediator_advertise_for_dcutr: bool,
    pub rendezvous_client_enabled: bool,
    pub rendezvous_server_enabled: bool,
}

impl ResolvedNodeConfig {
    pub fn from_config(cfg: &NodeConfig) -> Self {
        Self::try_from_config(cfg).expect("node config should resolve")
    }

    pub fn try_from_config(cfg: &NodeConfig) -> Result<Self, crate::common::error::NetError> {
        let environment = cfg.environment_report();
        super::capabilities::resolve_node_config(cfg, &environment)
    }

    pub fn from_config_and_environment(
        cfg: &NodeConfig,
        environment: &super::environment::EnvironmentReport,
    ) -> Self {
        Self::try_from_config_and_environment(cfg, environment)
            .expect("node config should resolve")
    }

    pub fn try_from_config_and_environment(
        cfg: &NodeConfig,
        environment: &super::environment::EnvironmentReport,
    ) -> Result<Self, crate::common::error::NetError> {
        super::capabilities::resolve_node_config(cfg, environment)
    }

    pub(crate) fn from_effective_config(
        profile: NodeProfile,
        role: NodeRole,
        effective: NodeConfig,
    ) -> Self {
        let mut enabled_behaviours = BehaviourSet::for_role(role, &effective);
        enabled_behaviours.dcutr = effective.dcutr.enabled && enabled_behaviours.relay_client;
        let has_relay_peers = !effective.relay_peers.is_empty();
        let lite_role = matches!(role, NodeRole::Lite | NodeRole::MobileLite);
        let mobile_lite = matches!(role, NodeRole::MobileLite);
        let relay_discovery_enabled = effective.discovery.relay_discovery.enabled
            && enabled_behaviours.relay_client
            && (lite_role || has_relay_peers);

        Self {
            profile,
            role,
            relay_server_enabled: enabled_behaviours.relay_server,
            mediator_enabled: effective.mediator.enabled,
            mediator_advertise_for_dcutr: effective.mediator.advertise_for_dcutr,
            rendezvous_client_enabled: enabled_behaviours.rendezvous_client,
            rendezvous_server_enabled: enabled_behaviours.rendezvous_server,
            reserve_configured_relays: effective.reserve_configured_relays,
            should_reserve_configured_relays: effective.reserve_configured_relays
                && has_relay_peers
                && enabled_behaviours.relay_client,
            should_seed_relay_peers: !effective.reserve_configured_relays
                && has_relay_peers
                && enabled_behaviours.relay_client,
            should_listen: !effective.listen_addresses.is_empty() && !mobile_lite,
            listen_addresses: effective.listen_addresses,
            relay_peers: effective.relay_peers,
            relay_discovery_enabled,
            relay_discovery_min_reservations: effective.discovery.relay_discovery.min_reservations,
            relay_discovery_max_reservations: effective.discovery.relay_discovery.max_reservations,
            dcutr_enabled: enabled_behaviours.dcutr,
            dcutr_attempt_after_relay_connection: effective.dcutr.attempt_after_relay_connection,
            dcutr_keep_relay_fallback: effective.dcutr.keep_relay_fallback,
            dcutr_retry_interval_secs: effective.dcutr.retry_interval_secs,
            dcutr_max_attempts_per_peer: effective.dcutr.max_attempts_per_peer,
            enabled_behaviours,
        }
    }
}
