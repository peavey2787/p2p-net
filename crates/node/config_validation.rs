//! Node configuration validation and multiaddr parsing helpers.

use crate::common::error::{config_error, NetError};
use crate::connectivity::dns::DnsaddrConfig;

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

use super::config::NodeConfig;

pub(crate) fn validate_node_config(cfg: &NodeConfig) -> Result<(), NetError> {
    if cfg.heartbeat_interval_secs == 0 {
        return Err(config_error("heartbeat_interval_secs must be at least 1"));
    }
    if cfg.identity_key_path.trim().is_empty() {
        return Err(config_error("identity_key_path must not be empty"));
    }

    cfg.dnsaddr.validate()?;
    validate_listen_addrs("listen_addresses", &cfg.listen_addresses)?;
    validate_peer_addrs("bootstrap_peers", &cfg.bootstrap_peers, true)?;
    validate_dnsaddr_use("bootstrap_peers", &cfg.bootstrap_peers, &cfg.dnsaddr)?;
    validate_peer_addrs(
        "discovery.bootstrap_seed_peers",
        &cfg.discovery.bootstrap_seed_peers,
        true,
    )?;
    validate_dnsaddr_use(
        "discovery.bootstrap_seed_peers",
        &cfg.discovery.bootstrap_seed_peers,
        &cfg.dnsaddr,
    )?;
    validate_peer_addrs(
        "discovery.public_bootstrap.bootstrap_seed_peers",
        &cfg.discovery.public_bootstrap.bootstrap_seed_peers,
        true,
    )?;
    validate_dnsaddr_use(
        "discovery.public_bootstrap.bootstrap_seed_peers",
        &cfg.discovery.public_bootstrap.bootstrap_seed_peers,
        &cfg.dnsaddr,
    )?;
    validate_peer_addrs(
        "discovery.public_bootstrap.relay_peers",
        &cfg.discovery.public_bootstrap.relay_peers,
        true,
    )?;
    validate_dnsaddr_use(
        "discovery.public_bootstrap.relay_peers",
        &cfg.discovery.public_bootstrap.relay_peers,
        &cfg.dnsaddr,
    )?;
    validate_peer_addrs(
        "discovery.rendezvous_peers",
        &cfg.discovery.rendezvous_peers,
        true,
    )?;
    validate_dnsaddr_use(
        "discovery.rendezvous_peers",
        &cfg.discovery.rendezvous_peers,
        &cfg.dnsaddr,
    )?;
    validate_peer_addrs("relay_peers", &cfg.relay_peers, true)?;
    validate_dnsaddr_use("relay_peers", &cfg.relay_peers, &cfg.dnsaddr)?;
    cfg.discovery.validate()?;
    cfg.connection_limits.validate()?;
    cfg.message_security.validate()?;
    cfg.dcutr.validate()?;
    cfg.relay.validate()?;
    cfg.mediator.validate(&cfg.relay)?;
    Ok(())
}
pub(crate) fn parse_multiaddrs(
    field: &str,
    values: &[String],
) -> Result<Vec<Multiaddr>, NetError> {
    values
        .iter()
        .map(|raw| {
            raw.parse::<Multiaddr>().map_err(|err| {
                config_error(format!("{field} contains invalid multiaddr `{raw}`: {err}"))
            })
        })
        .collect()
}

fn validate_listen_addrs(field: &str, values: &[String]) -> Result<(), NetError> {
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
) -> Result<(), NetError> {
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
) -> Result<(), NetError> {
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
