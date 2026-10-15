//! TURN configuration: where browsers get relay servers (`client`), whether
//! this node runs a TURN server (`server`), and whether it hands out
//! credentials to peers (`broker`). ICE policy and STUN are not here; they
//! belong to `browser_webrtc`. The `turn` node profile switches the server
//! and broker on.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::common::error::{config_error, NetError};

use super::credentials::{
    DEFAULT_CREDENTIAL_TTL_SECS, MAX_CREDENTIAL_TTL_SECS, MIN_CREDENTIAL_TTL_SECS,
};
use super::provider::TurnProviderConfig;
use super::secret::SecretRef;

mod client;

pub use client::{DiscoveryTrust, TurnClientConfig, TurnSource};

/// Most ICE servers handed to one `RTCPeerConnection`; browsers slow down
/// gathering noticeably beyond a handful.
pub const MAX_ICE_SERVERS: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnConfig {
    pub client: TurnClientConfig,
    pub server: TurnServerConfig,
    pub broker: TurnBrokerConfig,
}

impl TurnConfig {
    pub fn validate(&self) -> Result<(), NetError> {
        self.client.validate()?;
        self.server.validate()?;
        self.broker.validate(self.server.enabled)
    }
}

/// Built-in TURN server (UDP). Credentials use the TURN REST API shared
/// secret scheme, so coturn-style tooling can mint them too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnServerConfig {
    pub enabled: bool,
    /// Local address the UDP listener and relay sockets bind to.
    pub bind_ip: String,
    pub listen_port: u16,
    /// Address handed to clients as the relay address. Defaults to the
    /// public IP found by `public_ip_probe`.
    pub public_ip: Option<String>,
    /// DNS name used in advertised `turn:` URLs instead of the IP (set it
    /// when browsers reach this node by name, e.g. through `wss://`).
    pub public_host: Option<String>,
    pub realm: String,
    /// Secret for minting credentials. Unset: a random secret per run, which
    /// is enough when this node's broker is the only credential issuer.
    pub shared_secret: Option<SecretRef>,
    /// Relay port range. Its size caps concurrent allocations; open it (UDP)
    /// in the firewall together with `listen_port`.
    pub relay_port_min: u16,
    pub relay_port_max: u16,
    /// Allow relaying to private, loopback and link-local addresses. Leave
    /// off on public servers so the relay cannot reach the operator's LAN.
    pub allow_private_peers: bool,
}

impl Default for TurnServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_ip: "0.0.0.0".to_string(),
            listen_port: 3478,
            public_ip: None,
            public_host: None,
            realm: "p2p-net".to_string(),
            shared_secret: None,
            relay_port_min: 49160,
            relay_port_max: 49359,
            allow_private_peers: false,
        }
    }
}

impl TurnServerConfig {
    fn validate(&self) -> Result<(), NetError> {
        if !self.enabled {
            return Ok(());
        }
        parse_ip("turn.server.bind_ip", &self.bind_ip)?;
        if let Some(public_ip) = &self.public_ip {
            parse_ip("turn.server.public_ip", public_ip)?;
        }
        if let Some(host) = &self.public_host {
            let valid = !host.is_empty()
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
            if !valid {
                return Err(config_error("turn.server.public_host must be a DNS name"));
            }
        }
        if self.listen_port == 0 {
            return Err(config_error("turn.server.listen_port must not be 0"));
        }
        if self.realm.trim().is_empty() {
            return Err(config_error("turn.server.realm must not be empty"));
        }
        if let Some(secret) = &self.shared_secret {
            secret.validate("turn.server.shared_secret")?;
        }
        if self.relay_port_min < 1024 || self.relay_port_min > self.relay_port_max {
            return Err(config_error(
                "turn.server relay ports need 1024 <= relay_port_min <= relay_port_max",
            ));
        }
        if (self.relay_port_min..=self.relay_port_max).contains(&self.listen_port) {
            return Err(config_error(
                "turn.server.listen_port must be outside the relay port range",
            ));
        }
        Ok(())
    }

    /// Concurrent allocation capacity implied by the relay port range.
    pub fn allocation_capacity(&self) -> usize {
        usize::from(self.relay_port_max.saturating_sub(self.relay_port_min)) + 1
    }
}

/// Credential broker: answers `/p2p-net/turn-credentials/1.0.0` requests from
/// peers (normally browsers) with short-lived TURN credentials from the
/// built-in server and/or an upstream provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnBrokerConfig {
    pub enabled: bool,
    pub provider: Option<TurnProviderConfig>,
    pub credential_ttl_secs: u64,
    pub max_requests_per_peer_per_minute: u32,
}

impl Default for TurnBrokerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            credential_ttl_secs: DEFAULT_CREDENTIAL_TTL_SECS,
            max_requests_per_peer_per_minute: 6,
        }
    }
}

impl TurnBrokerConfig {
    fn validate(&self, server_enabled: bool) -> Result<(), NetError> {
        if !self.enabled {
            return Ok(());
        }
        match &self.provider {
            Some(provider) => provider.validate("turn.broker.provider")?,
            None if !server_enabled => {
                return Err(config_error(
                    "turn.broker.enabled needs turn.broker.provider or turn.server.enabled",
                ))
            }
            None => {}
        }
        if !(1..=600).contains(&self.max_requests_per_peer_per_minute) {
            return Err(config_error(
                "turn.broker.max_requests_per_peer_per_minute must be between 1 and 600",
            ));
        }
        validate_ttl("turn.broker.credential_ttl_secs", self.credential_ttl_secs)
    }
}

fn validate_ttl(field: &str, ttl: u64) -> Result<(), NetError> {
    if !(MIN_CREDENTIAL_TTL_SECS..=MAX_CREDENTIAL_TTL_SECS).contains(&ttl) {
        return Err(config_error(format!(
            "{field} must be between {MIN_CREDENTIAL_TTL_SECS} and {MAX_CREDENTIAL_TTL_SECS}"
        )));
    }
    Ok(())
}

fn parse_ip(field: &str, raw: &str) -> Result<IpAddr, NetError> {
    raw.parse()
        .map_err(|_| config_error(format!("{field} `{raw}` is not an IP address")))
}
