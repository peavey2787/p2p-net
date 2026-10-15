//! Browser WebRTC (`/webrtc`, browser-to-browser) settings.
//!
//! WebRTC owns connection establishment and ICE policy: whether the upgrade
//! runs at all, whether direct (host/STUN) candidates may be used, and which
//! STUN servers find them. TURN relay servers come from `turn.client`;
//! [`BrowserIcePlan::compose`] merges them into each peer connection's ICE
//! configuration without TURN ever overriding the policy decided here.

use serde::{Deserialize, Serialize};

use crate::common::error::{config_error, NetError};
use crate::connectivity::ice::{urls_or_servers, IceServer, IceTransportPolicy};
use crate::connectivity::turn::TurnClientConfig;

/// Most STUN servers per peer connection; one or two are plenty.
pub const MAX_STUN_SERVERS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserWebRtcConfig {
    /// Negotiate browser-to-browser WebRTC over relayed connections.
    pub enabled: bool,
    /// `all` (default) or `relay` (TURN candidates only). Unset follows
    /// `direct_connections`. When both are set they must agree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ice_transport_policy: Option<IceTransportPolicy>,
    /// `false` forbids direct (host/STUN) candidates, the same as
    /// `ice_transport_policy: "relay"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_connections: Option<bool>,
    pub stun: StunConfig,
    /// Original `stun:`/`turn:` list (bare URLs or `RTCIceServer` objects).
    /// Still honoured while configs migrate: STUN URLs join `stun.servers`
    /// and TURN entries join `turn.client.servers`.
    #[serde(
        deserialize_with = "urls_or_servers",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub ice_servers: Vec<IceServer>,
}

impl Default for BrowserWebRtcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ice_transport_policy: None,
            direct_connections: None,
            stun: StunConfig::default(),
            ice_servers: Vec::new(),
        }
    }
}

/// STUN lets browsers learn their public address for direct connections.
/// No STUN server is contacted unless one is listed (or a TURN source
/// supplies one); `enabled: false` also strips STUN URLs TURN sources offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StunConfig {
    pub enabled: bool,
    /// `stun:`/`stuns:` URLs.
    pub servers: Vec<String>,
}

impl Default for StunConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            servers: Vec::new(),
        }
    }
}

impl BrowserWebRtcConfig {
    /// The ICE policy in force. `relay` wins: a relay-only request from
    /// either field is never weakened (conflicts are rejected by `validate`).
    pub fn effective_policy(&self) -> IceTransportPolicy {
        let relay_only = self.ice_transport_policy == Some(IceTransportPolicy::Relay)
            || self.direct_connections == Some(false);
        if relay_only {
            IceTransportPolicy::Relay
        } else {
            IceTransportPolicy::All
        }
    }

    /// STUN servers from `stun.servers` and the original `ice_servers` list.
    pub fn stun_servers(&self) -> Vec<IceServer> {
        self.stun
            .servers
            .iter()
            .map(IceServer::stun)
            .chain(
                self.ice_servers
                    .iter()
                    .cloned()
                    .filter_map(IceServer::stun_only),
            )
            .collect()
    }

    /// `turn.client` with the legacy TURN entries of `ice_servers` added to
    /// its `self_hosted` servers, so they keep working unchanged.
    pub fn with_legacy_turn(&self, client: &TurnClientConfig) -> TurnClientConfig {
        let mut client = client.clone();
        client.servers.extend(
            self.ice_servers
                .iter()
                .cloned()
                .filter_map(IceServer::turn_only),
        );
        client
    }

    pub fn plan(&self) -> BrowserIcePlan {
        BrowserIcePlan {
            policy: self.effective_policy(),
            stun_enabled: self.stun.enabled,
            stun_servers: self.stun_servers(),
        }
    }

    pub fn validate(&self) -> Result<(), NetError> {
        if !self.enabled && self.has_settings() {
            return Err(config_error(
                "browser_webrtc.enabled is false, so its ICE, STUN and ice_servers settings would be ignored; remove them",
            ));
        }
        match (self.ice_transport_policy, self.direct_connections) {
            (Some(IceTransportPolicy::Relay), Some(true)) => Err(config_error(
                "browser_webrtc.ice_transport_policy is `relay` but direct_connections is true; relay-only ICE never connects directly",
            )),
            (Some(IceTransportPolicy::All), Some(false)) => Err(config_error(
                "browser_webrtc.direct_connections is false but ice_transport_policy is `all`; use `relay` (or leave it unset)",
            )),
            _ => self.validate_servers(),
        }
    }

    fn validate_servers(&self) -> Result<(), NetError> {
        for (index, url) in self.stun.servers.iter().enumerate() {
            let field = format!("browser_webrtc.stun.servers[{index}]");
            let server = IceServer::stun(url);
            server.validate(&field)?;
            if server.is_turn() {
                return Err(config_error(format!(
                    "{field}: TURN servers belong in turn.client.servers"
                )));
            }
        }
        for (index, server) in self.ice_servers.iter().enumerate() {
            server.validate(&format!("browser_webrtc.ice_servers[{index}]"))?;
        }
        let stun_count = self.stun_servers().len();
        if !self.stun.enabled && stun_count > 0 {
            return Err(config_error(
                "browser_webrtc.stun.enabled is false but STUN servers are listed",
            ));
        }
        if stun_count > MAX_STUN_SERVERS {
            return Err(config_error(format!(
                "browser_webrtc lists more than {MAX_STUN_SERVERS} STUN servers"
            )));
        }
        Ok(())
    }

    fn has_settings(&self) -> bool {
        self.ice_transport_policy.is_some()
            || self.direct_connections.is_some()
            || !self.stun.servers.is_empty()
            || !self.ice_servers.is_empty()
    }
}

/// Resolved WebRTC ICE settings that turn TURN relay servers into the
/// `iceServers` of a browser peer connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserIcePlan {
    pub policy: IceTransportPolicy,
    pub stun_enabled: bool,
    pub stun_servers: Vec<IceServer>,
}

impl BrowserIcePlan {
    /// STUN first (unless relay-only, which never uses it), then TURN.
    /// STUN URLs offered by TURN sources are dropped when STUN is disabled or
    /// ICE is relay-only, so no STUN server learns this browser's address.
    pub fn compose(&self, turn_servers: &[IceServer]) -> Vec<IceServer> {
        let direct = self.policy == IceTransportPolicy::All;
        let keep_stun = direct && self.stun_enabled;
        let stun = self.stun_servers.iter().filter(|_| keep_stun).cloned();
        let turn = turn_servers.iter().cloned().filter_map(|server| {
            if keep_stun {
                Some(server)
            } else {
                server.turn_only()
            }
        });
        let mut out: Vec<IceServer> = Vec::new();
        for server in stun.chain(turn) {
            if !out.iter().any(|existing| existing.urls == server.urls) {
                out.push(server);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(host: &str) -> IceServer {
        IceServer {
            urls: vec![
                format!("stun:{host}:3478"),
                format!("turn:{host}:3478?transport=udp"),
            ],
            username: Some("u".to_string()),
            credential: Some("c".to_string()),
        }
    }

    fn with_stun() -> BrowserWebRtcConfig {
        BrowserWebRtcConfig {
            stun: StunConfig {
                enabled: true,
                servers: vec!["stun:stun.example.org:3478".to_string()],
            },
            ..BrowserWebRtcConfig::default()
        }
    }

    #[test]
    fn direct_with_turn_fallback_keeps_stun_then_turn() {
        let ice = with_stun().plan().compose(&[turn("t.example.org")]);
        assert_eq!(ice.len(), 2);
        assert_eq!(ice[0], IceServer::stun("stun:stun.example.org:3478"));
        assert_eq!(ice[1], turn("t.example.org"));
    }

    #[test]
    fn relay_only_drops_every_stun_url() {
        for cfg in [
            BrowserWebRtcConfig {
                ice_transport_policy: Some(IceTransportPolicy::Relay),
                ..with_stun()
            },
            BrowserWebRtcConfig {
                direct_connections: Some(false),
                ..with_stun()
            },
        ] {
            cfg.validate().expect("relay-only is valid");
            let ice = cfg.plan().compose(&[turn("t.example.org")]);
            assert_eq!(cfg.effective_policy(), IceTransportPolicy::Relay);
            assert_eq!(ice.len(), 1);
            assert!(ice[0].urls.iter().all(|url| url.starts_with("turn:")));
        }
    }

    #[test]
    fn disabling_stun_strips_turn_offered_stun_urls() {
        let cfg = BrowserWebRtcConfig {
            stun: StunConfig {
                enabled: false,
                servers: Vec::new(),
            },
            ..BrowserWebRtcConfig::default()
        };
        let ice = cfg.plan().compose(&[turn("t.example.org")]);
        assert_eq!(ice[0].urls, vec!["turn:t.example.org:3478?transport=udp"]);
        assert_eq!(cfg.effective_policy(), IceTransportPolicy::All);
    }

    #[test]
    fn conflicting_policies_are_rejected() {
        for (policy, direct) in [
            (IceTransportPolicy::Relay, true),
            (IceTransportPolicy::All, false),
        ] {
            let cfg = BrowserWebRtcConfig {
                ice_transport_policy: Some(policy),
                direct_connections: Some(direct),
                ..BrowserWebRtcConfig::default()
            };
            assert!(cfg.validate().is_err(), "{policy:?} with direct={direct}");
        }
        let stun_off_with_servers = BrowserWebRtcConfig {
            stun: StunConfig {
                enabled: false,
                ..with_stun().stun
            },
            ..BrowserWebRtcConfig::default()
        };
        assert!(stun_off_with_servers.validate().is_err());
        let disabled_with_settings = BrowserWebRtcConfig {
            enabled: false,
            ..with_stun()
        };
        assert!(disabled_with_settings.validate().is_err());
    }

    #[test]
    fn legacy_ice_servers_split_into_stun_and_self_hosted_turn() {
        let cfg: BrowserWebRtcConfig = serde_json::from_str(
            r#"{"ice_servers": ["stun:legacy.example.org",
                {"urls": "turn:legacy.example.org", "username": "u", "credential": "c"}],
                "ice_transport_policy": "all"}"#,
        )
        .expect("legacy config parses");
        cfg.validate().expect("legacy config is valid");
        assert_eq!(
            cfg.stun_servers(),
            vec![IceServer::stun("stun:legacy.example.org")]
        );
        let client = cfg.with_legacy_turn(&TurnClientConfig::default());
        assert_eq!(client.servers.len(), 1);
        assert!(client.servers[0].is_turn());

        let bare_turn: BrowserWebRtcConfig =
            serde_json::from_str(r#"{"ice_servers": ["turn:legacy.example.org"]}"#)
                .expect("parses");
        assert!(
            bare_turn.validate().is_err(),
            "TURN without credentials is reported, not dropped"
        );
    }
}
