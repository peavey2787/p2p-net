//! `turn.client`: where a browser gets TURN relay servers and credentials.
//! It never decides ICE policy or STUN; `browser_webrtc` owns those and
//! composes these relay servers into each peer connection.

use serde::{Deserialize, Serialize};

use crate::common::error::{config_error, NetError};
use crate::connectivity::ice::IceServer;

use super::super::credentials::DEFAULT_CREDENTIAL_TTL_SECS;
use super::super::provider::{HttpProviderConfig, CLOUDFLARE_TURN_HOST};
use super::{validate_ttl, MAX_ICE_SERVERS};

const DEFAULT_MAX_SERVERS: usize = 4;

/// One kind of TURN source, in the order the client prefers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnSource {
    /// TURN servers advertised by p2p-net peers through their credential
    /// broker (`/p2p-net/turn-credentials/1.0.0`). Not Circuit Relay.
    Libp2p,
    /// Static entries in `turn.client.servers` (your own coturn or p2p-net
    /// TURN node with fixed credentials).
    SelfHosted,
    /// A Cloudflare Realtime TURN credential endpoint (`turn.client.cloudflare`,
    /// e.g. the Worker in `providers/turn/cloudflare`).
    Cloudflare,
    /// Any other HTTPS credential endpoint (`turn.client.http`).
    Http,
}

impl TurnSource {
    /// Automatic order: your own servers, then servers found on the network,
    /// then endpoints (which may bill per use).
    const AUTO_ORDER: [Self; 4] = [Self::SelfHosted, Self::Libp2p, Self::Http, Self::Cloudflare];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Libp2p => "libp2p",
            Self::SelfHosted => "self_hosted",
            Self::Cloudflare => "cloudflare",
            Self::Http => "http",
        }
    }
}

/// Which broker peers' TURN servers a browser accepts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryTrust {
    /// Only servers on the host this node dialed the broker at, or on a
    /// trusted provider host (default).
    #[default]
    SameHost,
    /// Servers from any broker peer.
    Any,
}

/// TURN relay sources for browser WebRTC.
///
/// Precedence: `enabled: false` turns every source off. Otherwise `providers`,
/// when set, is the exact ordered source list; when unset, every configured
/// source is used in [`TurnSource::AUTO_ORDER`], with `libp2p` included
/// while `auto_discover` is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnClientConfig {
    pub enabled: bool,
    /// Ask p2p-net broker peers for TURN servers (the `libp2p` source).
    pub auto_discover: bool,
    /// Explicit source order; `None` selects sources automatically.
    pub providers: Option<Vec<TurnSource>>,
    /// Static TURN entries (the `self_hosted` source).
    pub servers: Vec<IceServer>,
    /// Cloudflare credential endpoint. Browsers never hold the Cloudflare API
    /// token; it stays in the Worker or on a broker node.
    pub cloudflare: Option<HttpProviderConfig>,
    pub http: Option<HttpProviderConfig>,
    pub discovery_trust: DiscoveryTrust,
    /// Extra hosts accepted from broker peers under `same_host`.
    pub trusted_hosts: Vec<String>,
    /// Most TURN entries per peer connection.
    pub max_servers: usize,
    /// Lifetime requested from endpoints and brokers.
    pub credential_ttl_secs: u64,
}

impl Default for TurnClientConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_discover: true,
            providers: None,
            servers: Vec::new(),
            cloudflare: None,
            http: None,
            discovery_trust: DiscoveryTrust::SameHost,
            trusted_hosts: Vec::new(),
            max_servers: DEFAULT_MAX_SERVERS,
            credential_ttl_secs: DEFAULT_CREDENTIAL_TTL_SECS,
        }
    }
}

impl TurnClientConfig {
    /// Active sources in preference order.
    pub fn sources(&self) -> Vec<TurnSource> {
        if !self.enabled {
            return Vec::new();
        }
        match &self.providers {
            Some(explicit) => explicit.clone(),
            None => TurnSource::AUTO_ORDER
                .into_iter()
                .filter(|source| self.is_configured(*source))
                .collect(),
        }
    }

    /// True when the config names any TURN source beyond the defaults, so
    /// turning TURN or browser WebRTC off would silently drop it.
    pub fn has_explicit_sources(&self) -> bool {
        self.providers.is_some()
            || [
                TurnSource::SelfHosted,
                TurnSource::Cloudflare,
                TurnSource::Http,
            ]
            .into_iter()
            .any(|source| self.is_configured(source))
    }

    /// The endpoint behind an endpoint source.
    pub fn endpoint(&self, source: TurnSource) -> Option<&HttpProviderConfig> {
        match source {
            TurnSource::Cloudflare => self.cloudflare.as_ref(),
            TurnSource::Http => self.http.as_ref(),
            TurnSource::Libp2p | TurnSource::SelfHosted => None,
        }
    }

    /// How far broker-offered servers are trusted; `None` when the `libp2p`
    /// source is off.
    pub fn discovery(&self) -> Option<DiscoveryTrust> {
        self.sources()
            .contains(&TurnSource::Libp2p)
            .then_some(self.discovery_trust)
    }

    /// Hosts accepted from broker peers under `same_host`, lower-cased.
    pub fn accepted_provider_hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self
            .trusted_hosts
            .iter()
            .map(|host| host.trim().to_ascii_lowercase())
            .collect();
        hosts.push(CLOUDFLARE_TURN_HOST.to_string());
        hosts.sort();
        hosts.dedup();
        hosts
    }

    pub(super) fn validate(&self) -> Result<(), NetError> {
        if !self.enabled && self.has_explicit_sources() {
            return Err(config_error(
                "turn.client.enabled is false but TURN sources are configured; remove them or enable the client",
            ));
        }
        for (index, server) in self.servers.iter().enumerate() {
            let field = format!("turn.client.servers[{index}]");
            server.validate(&field)?;
            if !server.is_turn() {
                return Err(config_error(format!(
                    "{field} has no turn:/turns: url; STUN servers belong in browser_webrtc.stun.servers"
                )));
            }
        }
        for source in [TurnSource::Cloudflare, TurnSource::Http] {
            if let Some(endpoint) = self.endpoint(source) {
                validate_browser_endpoint(source, endpoint)?;
            }
        }
        if let Some(explicit) = &self.providers {
            self.validate_explicit_sources(explicit)?;
        }
        if !(1..=MAX_ICE_SERVERS).contains(&self.max_servers) {
            return Err(config_error(format!(
                "turn.client.max_servers must be between 1 and {MAX_ICE_SERVERS}"
            )));
        }
        if self.trusted_hosts.iter().any(|host| host.trim().is_empty()) {
            return Err(config_error(
                "turn.client.trusted_hosts must not contain empty hosts",
            ));
        }
        validate_ttl("turn.client.credential_ttl_secs", self.credential_ttl_secs)
    }

    /// An explicit list must be non-empty, unique, name only configured
    /// sources, and include every configured one (nothing is dropped silently).
    fn validate_explicit_sources(&self, explicit: &[TurnSource]) -> Result<(), NetError> {
        if explicit.is_empty() {
            return Err(config_error(
                "turn.client.providers must not be empty; set turn.client.enabled to false to use no TURN",
            ));
        }
        let mut seen = explicit.to_vec();
        seen.sort();
        seen.dedup();
        if seen.len() != explicit.len() {
            return Err(config_error("turn.client.providers lists a source twice"));
        }
        for source in explicit {
            if !self.is_configured(*source) {
                return Err(config_error(format!(
                    "turn.client.providers lists `{}`, which is not configured{}",
                    source.as_str(),
                    missing_hint(*source)
                )));
            }
        }
        for source in [
            TurnSource::SelfHosted,
            TurnSource::Cloudflare,
            TurnSource::Http,
        ] {
            if self.is_configured(source) && !explicit.contains(&source) {
                return Err(config_error(format!(
                    "turn.client.{} is configured but `{}` is missing from turn.client.providers",
                    config_key(source),
                    source.as_str()
                )));
            }
        }
        Ok(())
    }

    fn is_configured(&self, source: TurnSource) -> bool {
        match source {
            TurnSource::Libp2p => self.auto_discover,
            TurnSource::SelfHosted => !self.servers.is_empty(),
            TurnSource::Cloudflare | TurnSource::Http => self.endpoint(source).is_some(),
        }
    }
}

/// Client endpoints are fetched by browsers, so they must not carry secrets.
fn validate_browser_endpoint(
    source: TurnSource,
    endpoint: &HttpProviderConfig,
) -> Result<(), NetError> {
    let field = format!("turn.client.{}", config_key(source));
    if endpoint.bearer_token.is_some() {
        return Err(config_error(format!(
            "{field}.bearer_token would be shipped to every browser; protect the endpoint with \
             ALLOWED_ORIGINS and rate limits, or put the provider on a broker node (turn.broker.provider)"
        )));
    }
    endpoint.validate(&field)
}

fn config_key(source: TurnSource) -> &'static str {
    match source {
        TurnSource::Libp2p => "auto_discover",
        TurnSource::SelfHosted => "servers",
        TurnSource::Cloudflare => "cloudflare",
        TurnSource::Http => "http",
    }
}

fn missing_hint(source: TurnSource) -> &'static str {
    match source {
        TurnSource::Libp2p => " (turn.client.auto_discover is false)",
        TurnSource::SelfHosted => " (turn.client.servers is empty)",
        TurnSource::Cloudflare => {
            " (set turn.client.cloudflare.url to a credential endpoint such as providers/turn/cloudflare)"
        }
        TurnSource::Http => " (set turn.client.http.url)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(host: &str) -> IceServer {
        IceServer {
            urls: vec![format!("turn:{host}:3478")],
            username: Some("u".to_string()),
            credential: Some("c".to_string()),
        }
    }

    #[test]
    fn automatic_sources_follow_configuration_in_preference_order() {
        let mut cfg = TurnClientConfig::default();
        assert_eq!(cfg.sources(), vec![TurnSource::Libp2p]);
        cfg.servers = vec![turn("turn.example.org")];
        cfg.cloudflare = Some(HttpProviderConfig::new("https://w.example.dev/ice-servers"));
        assert_eq!(
            cfg.sources(),
            vec![
                TurnSource::SelfHosted,
                TurnSource::Libp2p,
                TurnSource::Cloudflare
            ]
        );
        cfg.auto_discover = false;
        assert_eq!(cfg.discovery(), None);
        cfg.enabled = false;
        assert!(cfg.sources().is_empty());
    }

    #[test]
    fn explicit_sources_are_exact_and_checked() {
        let mut cfg = TurnClientConfig {
            servers: vec![turn("turn.example.org")],
            providers: Some(vec![TurnSource::Libp2p, TurnSource::SelfHosted]),
            ..TurnClientConfig::default()
        };
        cfg.validate().expect("both sources configured");
        assert_eq!(
            cfg.sources(),
            vec![TurnSource::Libp2p, TurnSource::SelfHosted]
        );

        cfg.providers = Some(vec![TurnSource::Libp2p]);
        assert!(cfg.validate().is_err(), "configured servers left out");
        cfg.providers = Some(vec![TurnSource::SelfHosted, TurnSource::SelfHosted]);
        assert!(cfg.validate().is_err(), "duplicate source");
        cfg.providers = Some(vec![TurnSource::SelfHosted, TurnSource::Cloudflare]);
        assert!(cfg.validate().is_err(), "cloudflare not configured");
        cfg.providers = Some(Vec::new());
        assert!(cfg.validate().is_err(), "empty list");
        cfg.auto_discover = false;
        cfg.providers = Some(vec![TurnSource::Libp2p, TurnSource::SelfHosted]);
        assert!(
            cfg.validate().is_err(),
            "libp2p listed while auto_discover is off"
        );
    }

    #[test]
    fn disabled_client_rejects_configured_sources() {
        let cfg = TurnClientConfig {
            enabled: false,
            servers: vec![turn("turn.example.org")],
            ..TurnClientConfig::default()
        };
        assert!(cfg.validate().is_err());
        TurnClientConfig {
            enabled: false,
            ..TurnClientConfig::default()
        }
        .validate()
        .expect("plain off");
    }

    #[test]
    fn servers_must_be_turn_and_endpoints_must_not_hold_secrets() {
        let stun = TurnClientConfig {
            servers: vec![IceServer::stun("stun:stun.example.org")],
            ..TurnClientConfig::default()
        };
        assert!(stun.validate().is_err());
        let mut endpoint = HttpProviderConfig::new("https://w.example.dev/ice-servers");
        endpoint.bearer_token = Some(crate::connectivity::turn::SecretRef::Env(
            "TOKEN".to_string(),
        ));
        let secret = TurnClientConfig {
            http: Some(endpoint),
            ..TurnClientConfig::default()
        };
        assert!(secret.validate().is_err());
    }
}
