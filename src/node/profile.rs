//! Profile and capability resolution for the standalone node.
//!
//! Phase 1 introduces explicit user-facing profiles and a resolved capability
//! view without forcing every caller to understand individual libp2p behaviours.
//! Phase 2 adds advisory environment detection so callers can resolve `Auto`
//! against platform/reachability information without changing roles mid-run.

use serde::{Deserialize, Serialize};

use super::types::NodeConfig;

/// User-facing node profile. `Auto` preserves the current all-in-one defaults
/// until the environment detector and capability resolver land in later phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeProfile {
    /// Preserve current defaults for backward compatibility, then resolve to a
    /// concrete role using only explicit config flags.
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

    /// Apply phase-1 profile defaults to a clone of the raw config. This keeps
    /// old configs backward-compatible while making explicit profiles meaningful.
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

/// Phase-3 resolved view of user config. Runtime code should consume this
/// single policy object instead of re-deciding what each profile means.
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
        let enabled_behaviours = BehaviourSet::for_role(role, &effective);
        let has_relay_peers = !effective.relay_peers.is_empty();
        let mobile_lite = matches!(role, NodeRole::MobileLite);

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
            enabled_behaviours,
        }
    }
}
