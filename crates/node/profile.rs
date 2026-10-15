//! Profile and capability resolution for the standalone node.
//!
//! User-facing profiles and resolved capability policy for the standalone node.
//!
//! Callers choose a high-level `NodeProfile`; the central resolver turns it into
//! a concrete `NodeRole` and a libp2p-shaped `BehaviourSet`.

use serde::{Deserialize, Serialize};

use super::config::NodeConfig;
use crate::connectivity::connection_strategy::TransportCapabilities;

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
    /// Full node running the built-in TURN server and handing out short-lived
    /// TURN credentials to peers (browsers) over libp2p.
    Turn,
    /// Lite profile with conservative assumptions for mobile/tablet platforms.
    MobileLite,
}

impl NodeProfile {
    /// Every profile, in the order shown to users.
    pub const ALL: [Self; 9] = [
        Self::Auto,
        Self::Full,
        Self::Lite,
        Self::Relay,
        Self::Mediator,
        Self::Rendezvous,
        Self::Bootstrap,
        Self::Turn,
        Self::MobileLite,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Lite => "lite",
            Self::Relay => "relay",
            Self::Mediator => "mediator",
            Self::Rendezvous => "rendezvous",
            Self::Bootstrap => "bootstrap",
            Self::Turn => "turn",
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
            Self::Turn => {
                cfg.turn.server.enabled = true;
                cfg.turn.broker.enabled = true;
            }
        }
    }
}

impl std::str::FromStr for NodeProfile {
    type Err = String;

    /// Parse a profile name such as `turn` (the JSON spelling).
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|profile| profile.as_str() == name)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|p| p.as_str()).collect();
                format!(
                    "unknown profile `{name}`; choose one of: {}",
                    names.join(", ")
                )
            })
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
    Turn,
    MobileLite,
    /// Browser/WASM role: outbound browser transports, DHT/relay/rendezvous clients only.
    WasmLite,
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
            Self::Turn => "turn",
            Self::MobileLite => "mobile_lite",
            Self::WasmLite => "wasm_lite",
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
            NodeRole::Relay
                | NodeRole::Mediator
                | NodeRole::Rendezvous
                | NodeRole::Bootstrap
                | NodeRole::Turn
        );
        let lite = matches!(
            role,
            NodeRole::Lite | NodeRole::MobileLite | NodeRole::WasmLite
        );

        Self {
            gossipsub: true,
            kademlia_client: true,
            kademlia_server: !lite,
            autonat: !matches!(role, NodeRole::WasmLite),
            dcutr: !matches!(role, NodeRole::WasmLite),
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
    pub should_reserve_selected_relays: bool,
    pub should_seed_relay_peers: bool,
    pub should_seed_selected_relays: bool,
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
    /// Run the built-in TURN server.
    pub turn_server_enabled: bool,
    /// Answer TURN credential requests from peers.
    pub turn_broker_enabled: bool,
    /// Ask broker peers for TURN credentials (browser WebRTC with the
    /// `libp2p` TURN source only).
    pub turn_network_client_enabled: bool,
    pub transport_capabilities: TransportCapabilities,
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
        Self::try_from_config_and_environment(cfg, environment).expect("node config should resolve")
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
        // Narrow the role default: a role that forbids DCUtR (wasm_lite) never
        // gains it from `dcutr.enabled`, which defaults to true.
        enabled_behaviours.dcutr &= effective.dcutr.enabled && enabled_behaviours.relay_client;
        let has_relay_peers = !effective.relay_peers.is_empty();
        let has_public_relay_candidates =
            effective.discovery.public_bootstrap.has_relay_candidates();
        let has_public_dht_relay_discovery = effective.discovery.public_bootstrap.mode.is_enabled()
            && effective.discovery.relay_discovery.use_dht_relays;
        let lite_role = matches!(
            role,
            NodeRole::Lite | NodeRole::MobileLite | NodeRole::WasmLite
        );
        let no_public_listeners = matches!(role, NodeRole::MobileLite | NodeRole::WasmLite);
        let mut transport_capabilities = if matches!(role, NodeRole::WasmLite) {
            TransportCapabilities::browser()
        } else {
            TransportCapabilities::native()
        };
        if !effective.browser_webrtc.enabled {
            // In browsers the relay-assisted upgrade is the WebRTC signaling.
            transport_capabilities.browser_webrtc = false;
            transport_capabilities.relay_assisted_upgrade &= !matches!(role, NodeRole::WasmLite);
        }
        let turn = &effective.turn;
        let has_relay_selection_source = lite_role
            || has_relay_peers
            || has_public_relay_candidates
            || has_public_dht_relay_discovery;
        let relay_discovery_enabled = effective.discovery.relay_discovery.enabled
            && enabled_behaviours.relay_client
            && has_relay_selection_source;
        let should_listen = !no_public_listeners
            && effective
                .enabled_listen_addresses()
                .map(|addresses| !addresses.is_empty())
                .unwrap_or(false);

        Self {
            profile,
            role,
            relay_server_enabled: enabled_behaviours.relay_server,
            mediator_enabled: effective.mediator.enabled,
            mediator_advertise_for_dcutr: effective.mediator.advertise_for_dcutr,
            rendezvous_client_enabled: enabled_behaviours.rendezvous_client,
            rendezvous_server_enabled: enabled_behaviours.rendezvous_server,
            turn_server_enabled: turn.server.enabled && !no_public_listeners,
            turn_broker_enabled: turn.broker.enabled && !no_public_listeners,
            turn_network_client_enabled: transport_capabilities.browser_webrtc
                && turn.client.discovery().is_some(),
            reserve_configured_relays: effective.reserve_configured_relays,
            should_reserve_configured_relays: effective.reserve_configured_relays
                && has_relay_peers
                && enabled_behaviours.relay_client,
            should_reserve_selected_relays: effective.reserve_configured_relays
                && has_relay_selection_source
                && enabled_behaviours.relay_client,
            should_seed_relay_peers: !effective.reserve_configured_relays
                && has_relay_peers
                && enabled_behaviours.relay_client,
            should_seed_selected_relays: !effective.reserve_configured_relays
                && has_relay_selection_source
                && enabled_behaviours.relay_client,
            should_listen,
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
            transport_capabilities,
            enabled_behaviours,
        }
    }
}
