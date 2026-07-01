//! Profile and capability resolution for the standalone node.
//!
//! Phase 1 introduces explicit user-facing profiles and a resolved capability
//! view without forcing every caller to understand individual libp2p behaviours.
//! Phase 2 adds advisory environment detection so callers can resolve `Auto`
//! against platform/reachability information without changing roles mid-run.

use serde::{Deserialize, Serialize};

use super::environment::{EnvironmentReport, NetworkReachability};
use super::types::NodeConfig;

/// User-facing node profile. `Auto` preserves the current all-in-one defaults
/// until the environment detector and capability resolver land in later phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeProfile {
    /// Preserve current defaults for backward compatibility, then resolve to a
    /// concrete role using only explicit config flags.
    Auto,
    /// Reachable node that listens for inbound peers and participates directly.
    Full,
    /// Outbound-first node intended for NAT/CGNAT environments.
    Lite,
    /// Full node with Circuit Relay service enabled so it can mediate relayed
    /// connections and DCUtR attempts for other peers.
    Relay,
    /// Infrastructure node that enables rendezvous server capabilities.
    Rendezvous,
    /// Infrastructure node optimized for stable discovery/bootstrap addressing.
    Bootstrap,
    /// Lite profile with conservative assumptions for mobile/tablet platforms.
    MobileLite,
}

impl Default for NodeProfile {
    fn default() -> Self {
        Self::Auto
    }
}

impl NodeProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Lite => "lite",
            Self::Relay => "relay",
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
            NodeRole::Relay | NodeRole::Rendezvous | NodeRole::Bootstrap
        );
        let lite = matches!(role, NodeRole::Lite | NodeRole::MobileLite);

        Self {
            gossipsub: true,
            kademlia_client: true,
            kademlia_server: !lite,
            autonat: true,
            dcutr: true,
            relay_client: true,
            relay_server: matches!(role, NodeRole::Relay) || cfg.relay.enabled,
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

/// Phase-1 resolved view of user config. Later phases should make this the only
/// input to transport/behaviour construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedNodeConfig {
    pub profile: NodeProfile,
    pub role: NodeRole,
    pub enabled_behaviours: BehaviourSet,
    pub reserve_configured_relays: bool,
    pub listen_addresses: Vec<String>,
    pub relay_peers: Vec<String>,
    pub relay_server_enabled: bool,
    pub rendezvous_client_enabled: bool,
    pub rendezvous_server_enabled: bool,
}

impl ResolvedNodeConfig {
    pub fn from_config(cfg: &NodeConfig) -> Self {
        let mut effective = cfg.clone();
        effective.profile.apply_to(&mut effective);
        let role = resolve_role(&effective);
        Self::from_effective_config(cfg.profile, role, effective)
    }

    pub fn from_config_and_environment(cfg: &NodeConfig, environment: &EnvironmentReport) -> Self {
        let mut effective = cfg.clone();
        effective.profile.apply_to(&mut effective);
        let role = resolve_role_for_environment(&effective, environment);
        Self::from_effective_config(cfg.profile, role, effective)
    }

    fn from_effective_config(profile: NodeProfile, role: NodeRole, effective: NodeConfig) -> Self {
        let enabled_behaviours = BehaviourSet::for_role(role, &effective);

        Self {
            profile,
            role,
            relay_server_enabled: enabled_behaviours.relay_server,
            rendezvous_client_enabled: enabled_behaviours.rendezvous_client,
            rendezvous_server_enabled: enabled_behaviours.rendezvous_server,
            reserve_configured_relays: effective.reserve_configured_relays,
            listen_addresses: effective.listen_addresses,
            relay_peers: effective.relay_peers,
            enabled_behaviours,
        }
    }
}

pub(crate) fn resolve_role(cfg: &NodeConfig) -> NodeRole {
    match cfg.profile {
        NodeProfile::Auto => explicit_config_role(cfg).unwrap_or(NodeRole::Full),
        NodeProfile::Full => NodeRole::Full,
        NodeProfile::Lite => NodeRole::Lite,
        NodeProfile::Relay => NodeRole::Relay,
        NodeProfile::Rendezvous => NodeRole::Rendezvous,
        NodeProfile::Bootstrap => NodeRole::Bootstrap,
        NodeProfile::MobileLite => NodeRole::MobileLite,
    }
}

pub(crate) fn resolve_role_for_environment(
    cfg: &NodeConfig,
    environment: &EnvironmentReport,
) -> NodeRole {
    match cfg.profile {
        NodeProfile::Auto => explicit_config_role(cfg).unwrap_or_else(|| auto_role(environment)),
        NodeProfile::Full => NodeRole::Full,
        NodeProfile::Lite => NodeRole::Lite,
        NodeProfile::Relay => NodeRole::Relay,
        NodeProfile::Rendezvous => NodeRole::Rendezvous,
        NodeProfile::Bootstrap => NodeRole::Bootstrap,
        NodeProfile::MobileLite => NodeRole::MobileLite,
    }
}

fn explicit_config_role(cfg: &NodeConfig) -> Option<NodeRole> {
    if cfg.relay.enabled {
        Some(NodeRole::Relay)
    } else if cfg.discovery.rendezvous.server_enabled {
        Some(NodeRole::Rendezvous)
    } else {
        None
    }
}

fn auto_role(environment: &EnvironmentReport) -> NodeRole {
    if environment.platform.is_mobile() || environment.background_restricted {
        return NodeRole::MobileLite;
    }
    if environment.can_accept_inbound
        || matches!(environment.reachability, NetworkReachability::Public)
    {
        return NodeRole::Full;
    }
    if environment.likely_cgnat
        || matches!(
            environment.reachability,
            NetworkReachability::PrivateNat | NetworkReachability::CgnatLikely
        )
    {
        return NodeRole::Lite;
    }
    NodeRole::Full
}
