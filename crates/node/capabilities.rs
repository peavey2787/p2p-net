//! Central capability resolver for node profiles.
//!
//! This module is the single place that turns raw user config plus environment
//! facts into runtime capabilities. Runtime code consumes the returned
//! `ResolvedNodeConfig` instead of re-implementing profile decisions.

use crate::common::error::config_error_at;
use crate::common::error::NetError;

use super::environment::{EnvironmentReport, NetworkReachability};
use super::profile::{NodeProfile, NodeRole, ResolvedNodeConfig};
use super::types::NodeConfig;

/// Resolve raw user config plus an advisory environment report into one
/// validated capability view.
pub fn resolve_node_config(
    raw: &NodeConfig,
    environment: &EnvironmentReport,
) -> Result<ResolvedNodeConfig, NetError> {
    let effective = effective_config_for_resolution(raw);
    effective.discovery.relay_discovery.validate()?;
    effective.dcutr.validate()?;
    effective.mediator.validate(&effective.relay)?;
    let role = resolve_role_for_environment(&effective, environment);
    let resolved = ResolvedNodeConfig::from_effective_config(raw.profile, role, effective);
    validate_resolved_config(raw, environment, &resolved)?;
    Ok(resolved)
}

/// Apply the central resolver's decisions back onto a raw config clone. This
/// produces the effective runtime config used by transport, discovery, and
/// behaviour construction while keeping profile policy centralized.
pub fn apply_resolved_capabilities(raw: &NodeConfig, resolved: &ResolvedNodeConfig) -> NodeConfig {
    let mut cfg = raw.clone();
    cfg.profile.apply_to(&mut cfg);
    let mediator = cfg.mediator.clone();
    mediator.apply_to_relay(&mut cfg.relay);

    cfg.relay.enabled = resolved.enabled_behaviours.relay_server;
    cfg.dcutr.enabled = resolved.dcutr_enabled;
    cfg.discovery.rendezvous.client_enabled = resolved.enabled_behaviours.rendezvous_client;
    cfg.discovery.rendezvous.server_enabled = resolved.enabled_behaviours.rendezvous_server;
    cfg.reserve_configured_relays = resolved.reserve_configured_relays;
    if !resolved.should_listen {
        cfg.listen_addresses.clear();
    }

    cfg
}

fn effective_config_for_resolution(raw: &NodeConfig) -> NodeConfig {
    let mut effective = raw.clone();
    effective.profile.apply_to(&mut effective);
    let mediator = effective.mediator.clone();
    mediator.apply_to_relay(&mut effective.relay);
    effective
}

fn resolve_role_for_environment(cfg: &NodeConfig, environment: &EnvironmentReport) -> NodeRole {
    match cfg.profile {
        NodeProfile::Auto => explicit_config_role(cfg).unwrap_or(auto_role(environment)),
        NodeProfile::Full => NodeRole::Full,
        NodeProfile::Lite => NodeRole::Lite,
        NodeProfile::Relay => NodeRole::Relay,
        NodeProfile::Mediator => NodeRole::Mediator,
        NodeProfile::Rendezvous => NodeRole::Rendezvous,
        NodeProfile::Bootstrap => NodeRole::Bootstrap,
        NodeProfile::MobileLite => NodeRole::MobileLite,
    }
}

fn explicit_config_role(cfg: &NodeConfig) -> Option<NodeRole> {
    if cfg.mediator.enabled {
        Some(NodeRole::Mediator)
    } else if cfg.relay.enabled {
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

fn validate_resolved_config(
    raw: &NodeConfig,
    environment: &EnvironmentReport,
    resolved: &ResolvedNodeConfig,
) -> Result<(), NetError> {
    let role = resolved.role;
    let behaviours = &resolved.enabled_behaviours;

    if matches!(role, NodeRole::Lite | NodeRole::MobileLite) && behaviours.relay_server {
        return Err(config_error_at(
            "<capability-resolver>",
            "lite and mobile_lite profiles cannot enable relay server capability",
        ));
    }

    if matches!(role, NodeRole::Lite | NodeRole::MobileLite) && behaviours.rendezvous_server {
        return Err(config_error_at(
            "<capability-resolver>",
            "lite and mobile_lite profiles cannot enable rendezvous server capability",
        ));
    }

    if behaviours.dcutr && !behaviours.relay_client {
        return Err(config_error_at(
            "<capability-resolver>",
            "DCUtR requires relay client capability for relayed fallback",
        ));
    }

    if resolved.dcutr_enabled && !resolved.dcutr_keep_relay_fallback {
        return Err(config_error_at(
            "<capability-resolver>",
            "DCUtR policy must keep relay fallback enabled for production-safe lite connectivity",
        ));
    }

    if resolved.dcutr_enabled && resolved.dcutr_max_attempts_per_peer == 0 {
        return Err(config_error_at(
            "<capability-resolver>",
            "DCUtR max attempts per peer must be at least 1 when enabled",
        ));
    }

    if resolved.relay_discovery_enabled && !behaviours.relay_client {
        return Err(config_error_at(
            "<capability-resolver>",
            "relay discovery requires relay client capability",
        ));
    }

    if resolved.mediator_enabled && !behaviours.relay_server {
        return Err(config_error_at(
            "<capability-resolver>",
            "mediator capability requires relay server capability",
        ));
    }

    if resolved.mediator_enabled && !resolved.mediator_advertise_for_dcutr {
        return Err(config_error_at(
            "<capability-resolver>",
            "mediator.advertise_for_dcutr must be true for mediator role",
        ));
    }

    if behaviours.relay_server && environment.background_restricted {
        return Err(config_error_at(
            "<capability-resolver>",
            "relay server capability is not allowed in a background-restricted environment",
        ));
    }

    if behaviours.relay_server && raw.listen_addresses.is_empty() {
        return Err(config_error_at(
            "<capability-resolver>",
            "relay server capability requires at least one listen address",
        ));
    }

    if behaviours.relay_server && !environment.can_listen_tcp && !environment.can_listen_quic {
        return Err(config_error_at(
            "<capability-resolver>",
            "relay server capability requires TCP or QUIC listen support",
        ));
    }

    if matches!(role, NodeRole::MobileLite) && resolved.should_listen {
        return Err(config_error_at(
            "<capability-resolver>",
            "mobile_lite resolved policy must not require public listen sockets",
        ));
    }

    Ok(())
}
