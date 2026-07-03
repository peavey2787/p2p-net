//! Public IP probing and external address synthesis.
//!
//! libp2p can learn public reachability from Identify/AutoNAT, but consumer
//! app mode also needs a practical first-launch public-IP hint for dashboards
//! and direct external-address advertisement. This module keeps that optional
//! HTTP probe separate from node orchestration.

use std::net::IpAddr;
use std::time::Duration;

use crate::common::error::{config_error, NetError};
use crate::connectivity::addr::is_public_direct_addr;

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

const DEFAULT_PUBLIC_IP_ENDPOINTS: &[&str] = &[
    "https://api.ipify.org",
    "https://checkip.amazonaws.com",
    "https://ifconfig.me/ip",
];

/// Optional public-IP probe used by consumer default app mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PublicIpProbeConfig {
    /// Query HTTPS "what is my IP" endpoints during startup runtime.
    pub enabled: bool,
    /// Probe endpoints. Each endpoint must return only an IPv4/IPv6 address or
    /// an address with surrounding whitespace.
    pub endpoints: Vec<String>,
    /// Per-endpoint timeout in seconds.
    pub timeout_secs: u64,
    /// Synthesize public external multiaddrs from configured listen ports.
    pub advertise_listen_addresses: bool,
}

impl Default for PublicIpProbeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoints: DEFAULT_PUBLIC_IP_ENDPOINTS
                .iter()
                .map(|endpoint| (*endpoint).to_string())
                .collect(),
            timeout_secs: 2,
            advertise_listen_addresses: true,
        }
    }
}

impl PublicIpProbeConfig {
    pub(crate) fn validate(&self) -> Result<(), NetError> {
        if !self.enabled {
            return Ok(());
        }
        if self.endpoints.is_empty() {
            return Err(config_error(
                "public_ip_probe.enabled is true but public_ip_probe.endpoints is empty",
            ));
        }
        if self.timeout_secs == 0 || self.timeout_secs > 30 {
            return Err(config_error(
                "public_ip_probe.timeout_secs must be between 1 and 30 when enabled",
            ));
        }
        for endpoint in &self.endpoints {
            if !endpoint.starts_with("https://") {
                return Err(config_error(format!(
                    "public_ip_probe endpoint must use https: `{endpoint}`"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PublicIpProbeResult {
    pub(crate) status: String,
    pub(crate) public_ip: Option<String>,
    pub(crate) external_addresses: Vec<Multiaddr>,
    pub(crate) errors: Vec<String>,
}

impl PublicIpProbeResult {
    pub(crate) fn pulse_line(&self) -> Option<String> {
        if self.status == "disabled" {
            return None;
        }
        let public_ip = self.public_ip.as_deref().unwrap_or("unknown");
        if self.status == "public_ip_found" {
            return Some(format!(
                "public_ip_probe status=public_ip_found ip={public_ip} external_addrs={}",
                self.external_addresses.len()
            ));
        }
        Some(format!(
            "public_ip_probe status={} ip={} errors={}",
            self.status,
            public_ip,
            self.errors.len()
        ))
    }
}

pub(crate) async fn probe_public_addresses(
    cfg: PublicIpProbeConfig,
    listen_addresses: Vec<String>,
) -> PublicIpProbeResult {
    if !cfg.enabled {
        return PublicIpProbeResult {
            status: "disabled".to_string(),
            ..PublicIpProbeResult::default()
        };
    }

    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
        .build()
    else {
        return PublicIpProbeResult {
            status: "client_build_failed".to_string(),
            ..PublicIpProbeResult::default()
        };
    };

    let mut errors = Vec::new();
    for endpoint in &cfg.endpoints {
        match fetch_public_ip(&client, endpoint).await {
            Ok(ip) => {
                let external_addresses = if cfg.advertise_listen_addresses {
                    synthesize_external_addresses(ip, &listen_addresses)
                } else {
                    Vec::new()
                };
                let status = if external_addresses.is_empty() {
                    "public_ip_found_no_advertisable_listen_addrs"
                } else {
                    "public_ip_found"
                };
                return PublicIpProbeResult {
                    status: status.to_string(),
                    public_ip: Some(ip.to_string()),
                    external_addresses,
                    errors,
                };
            }
            Err(err) => errors.push(format!("{endpoint}: {err}")),
        }
    }

    PublicIpProbeResult {
        status: "failed".to_string(),
        errors,
        ..PublicIpProbeResult::default()
    }
}

async fn fetch_public_ip(client: &reqwest::Client, endpoint: &str) -> Result<IpAddr, String> {
    let text = client
        .get(endpoint)
        .send()
        .await
        .map_err(|err| err.to_string())?
        .error_for_status()
        .map_err(|err| err.to_string())?
        .text()
        .await
        .map_err(|err| err.to_string())?;
    text.trim()
        .parse::<IpAddr>()
        .map_err(|err| format!("invalid public IP response `{}`: {err}", text.trim()))
}

fn synthesize_external_addresses(ip: IpAddr, listen_addresses: &[String]) -> Vec<Multiaddr> {
    let mut addresses = Vec::new();
    for raw in listen_addresses {
        let Ok(addr) = raw.parse::<Multiaddr>() else {
            continue;
        };
        let Some(external) = rewrite_listen_ip(&addr, ip) else {
            continue;
        };
        if is_public_direct_addr(&external) && !addresses.contains(&external) {
            addresses.push(external);
        }
    }
    addresses
}

fn rewrite_listen_ip(addr: &Multiaddr, public_ip: IpAddr) -> Option<Multiaddr> {
    let mut rewritten = Multiaddr::empty();
    let mut replaced_ip = false;

    for protocol in addr.iter() {
        match (protocol, public_ip) {
            (Protocol::Ip4(listen_ip), IpAddr::V4(public_ip)) => {
                if listen_ip.is_loopback() {
                    return None;
                }
                rewritten.push(Protocol::Ip4(public_ip));
                replaced_ip = true;
            }
            (Protocol::Ip6(listen_ip), IpAddr::V6(public_ip)) => {
                if listen_ip.is_loopback() {
                    return None;
                }
                rewritten.push(Protocol::Ip6(public_ip));
                replaced_ip = true;
            }
            (Protocol::Ip4(_), IpAddr::V6(_)) | (Protocol::Ip6(_), IpAddr::V4(_)) => return None,
            (Protocol::Dns(_), _) | (Protocol::Dns4(_), _) | (Protocol::Dns6(_), _) => return None,
            (other, _) => rewritten.push(other),
        }
    }

    replaced_ip.then_some(rewritten)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn default_public_ip_probe_is_enabled_for_consumer_mode() {
        let cfg = PublicIpProbeConfig::default();

        assert!(cfg.enabled);
        assert!(!cfg.endpoints.is_empty());
        assert_eq!(cfg.timeout_secs, 2);
        assert!(cfg.advertise_listen_addresses);
    }

    #[test]
    fn public_ip_rewrites_wildcard_listen_ports() {
        let addrs = synthesize_external_addresses(
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            &[
                "/ip4/0.0.0.0/udp/4001/quic-v1".to_string(),
                "/ip4/0.0.0.0/tcp/4001".to_string(),
                "/ip4/127.0.0.1/tcp/4002/ws".to_string(),
            ],
        );

        assert_eq!(addrs.len(), 2);
        assert!(addrs
            .iter()
            .any(|addr| addr.to_string() == "/ip4/8.8.8.8/udp/4001/quic-v1"));
        assert!(addrs
            .iter()
            .any(|addr| addr.to_string() == "/ip4/8.8.8.8/tcp/4001"));
    }

    #[test]
    fn public_ip_rewrites_ipv6_wildcard_listen_ports() {
        let addrs = synthesize_external_addresses(
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            &["/ip6/::/tcp/4001".to_string()],
        );

        assert!(addrs.is_empty());
    }
}
