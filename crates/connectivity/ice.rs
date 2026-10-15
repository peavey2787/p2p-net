//! STUN/TURN ICE server entries in the W3C `RTCIceServer` shape, shared by
//! browser WebRTC (which owns ICE policy and STUN) and TURN (which supplies
//! relay servers and credentials).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::common::error::{config_error, NetError};

const MAX_ICE_URL_LEN: usize = 512;

/// One ICE server: `urls` plus, for TURN, the username/credential pair. The
/// JSON shape matches `RTCIceServer`, so `urls` may be a string or a list.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServer {
    #[serde(deserialize_with = "one_or_many")]
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

impl fmt::Debug for IceServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IceServer")
            .field("urls", &self.urls)
            .field("username", &self.username.as_ref().map(|_| "<redacted>"))
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl IceServer {
    /// A STUN-only entry.
    pub fn stun(url: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            username: None,
            credential: None,
        }
    }

    /// Reject unknown schemes, malformed hosts/ports, and TURN entries
    /// without credentials (browsers refuse to build such a configuration).
    pub fn validate(&self, field: &str) -> Result<(), NetError> {
        if self.urls.is_empty() {
            return Err(config_error(format!("{field}: urls must not be empty")));
        }
        for url in &self.urls {
            let parsed = IceUrl::parse(url)
                .map_err(|reason| config_error(format!("{field}: `{url}` {reason}")))?;
            if parsed.scheme.is_turn() && (self.username.is_none() || self.credential.is_none()) {
                return Err(config_error(format!(
                    "{field}: TURN url `{url}` needs username and credential"
                )));
            }
        }
        Ok(())
    }

    /// True when any URL is `turn:` or `turns:`.
    pub fn is_turn(&self) -> bool {
        self.parsed_urls().any(|url| url.scheme.is_turn())
    }

    /// Lower-cased hosts of all parseable URLs.
    pub fn hosts(&self) -> impl Iterator<Item = String> + '_ {
        self.parsed_urls().map(|url| url.host)
    }

    /// Drop URLs whose port is `port` (e.g. 53, which browsers block and
    /// which only stalls ICE gathering). Returns `None` when no URL is left.
    pub fn without_port(mut self, port: u16) -> Option<Self> {
        self.urls
            .retain(|url| IceUrl::parse(url).map_or(true, |parsed| parsed.port != Some(port)));
        (!self.urls.is_empty()).then_some(self)
    }

    /// Keep only `turn:`/`turns:` URLs. Returns `None` when no URL is left.
    pub fn turn_only(mut self) -> Option<Self> {
        self.urls
            .retain(|url| IceUrl::parse(url).is_ok_and(|parsed| parsed.scheme.is_turn()));
        (!self.urls.is_empty()).then_some(self)
    }

    /// Keep only `stun:`/`stuns:` URLs, without credentials. Returns `None`
    /// when no URL is left.
    pub fn stun_only(self) -> Option<Self> {
        let urls: Vec<String> = self
            .urls
            .into_iter()
            .filter(|url| IceUrl::parse(url).is_ok_and(|parsed| !parsed.scheme.is_turn()))
            .collect();
        (!urls.is_empty()).then_some(Self {
            urls,
            username: None,
            credential: None,
        })
    }

    /// Browser WebRTC form consumed by the signaling behaviour.
    pub fn to_webrtc(&self) -> libp2p_webrtc_websys::IceServer {
        libp2p_webrtc_websys::IceServer {
            urls: self.urls.clone(),
            username: self.username.clone(),
            credential: self.credential.clone(),
        }
    }

    fn parsed_urls(&self) -> impl Iterator<Item = IceUrl> + '_ {
        self.urls.iter().filter_map(|url| IceUrl::parse(url).ok())
    }
}

/// ICE transport policy: `all` (default) or `relay` (TURN candidates only;
/// hides local addresses and needs at least one TURN server to connect).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IceTransportPolicy {
    #[default]
    All,
    Relay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IceScheme {
    Stun,
    Stuns,
    Turn,
    Turns,
}

impl IceScheme {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "stun" => Some(Self::Stun),
            "stuns" => Some(Self::Stuns),
            "turn" => Some(Self::Turn),
            "turns" => Some(Self::Turns),
            _ => None,
        }
    }

    fn is_turn(self) -> bool {
        matches!(self, Self::Turn | Self::Turns)
    }
}

/// Parsed `scheme:host[:port][?transport=udp|tcp]` (RFC 7064/7065).
#[derive(Debug, Clone, PartialEq, Eq)]
struct IceUrl {
    scheme: IceScheme,
    host: String,
    port: Option<u16>,
}

impl IceUrl {
    fn parse(url: &str) -> Result<Self, &'static str> {
        if url.len() > MAX_ICE_URL_LEN {
            return Err("is too long");
        }
        let (scheme, rest) = url.split_once(':').ok_or("is missing a scheme")?;
        let scheme = IceScheme::parse(scheme).ok_or("must use stun:, stuns:, turn: or turns:")?;
        let (authority, query) = rest.split_once('?').unwrap_or((rest, ""));
        if !query.is_empty() && !matches!(query, "transport=udp" | "transport=tcp") {
            return Err("has an unsupported query (only transport=udp|tcp)");
        }
        if !query.is_empty() && !scheme.is_turn() {
            return Err("transport= is only valid on TURN urls");
        }
        let (host, port) = split_host_port(authority)?;
        if host.is_empty()
            || !host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        {
            return Err("has an invalid host");
        }
        Ok(Self {
            scheme,
            host: host.to_ascii_lowercase(),
            port,
        })
    }
}

fn split_host_port(authority: &str) -> Result<(&str, Option<u16>), &'static str> {
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, tail) = bracketed
            .split_once(']')
            .ok_or("has an unclosed IPv6 host")?;
        match tail {
            "" => (host, None),
            _ => (
                host,
                Some(tail.strip_prefix(':').ok_or("has an invalid port")?),
            ),
        }
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let port = port
        .map(|raw| raw.parse::<u16>().ok().filter(|port| *port != 0))
        .map(|port| port.ok_or("has an invalid port"))
        .transpose()?;
    Ok((host, port))
}

/// Accept either a single JSON value or a list of them (the `RTCIceServer`
/// convention for `urls`, also used by some providers for `iceServers`).
pub(crate) fn one_or_many<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        One(T),
        Many(Vec<T>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(value) => vec![value],
        OneOrMany::Many(values) => values,
    })
}

/// A list whose entries are `RTCIceServer` objects or bare URL strings (the
/// original `browser_webrtc.ice_servers` format).
pub(crate) fn urls_or_servers<'de, D>(deserializer: D) -> Result<Vec<IceServer>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Url(String),
        Server(IceServer),
    }
    Ok(Vec::<Entry>::deserialize(deserializer)?
        .into_iter()
        .map(|entry| match entry {
            Entry::Url(url) => IceServer {
                urls: vec![url],
                username: None,
                credential: None,
            },
            Entry::Server(server) => server,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_only_drops_stun_urls() {
        let mixed = IceServer {
            urls: vec![
                "stun:t.example.org".to_string(),
                "turn:t.example.org".to_string(),
            ],
            username: Some("u".to_string()),
            credential: Some("c".to_string()),
        };
        assert_eq!(
            mixed.clone().turn_only().expect("turn url").urls,
            vec!["turn:t.example.org"]
        );
        assert!(IceServer::stun("stun:s.example.org").turn_only().is_none());
        let stun = mixed.stun_only().expect("stun url");
        assert_eq!(stun, IceServer::stun("stun:t.example.org"));
        assert!(turn("turn:t.example.org").stun_only().is_none());
    }

    fn turn(url: &str) -> IceServer {
        IceServer {
            urls: vec![url.to_string()],
            username: Some("u".to_string()),
            credential: Some("c".to_string()),
        }
    }

    #[test]
    fn accepts_rtc_ice_server_json_with_string_or_list_urls() {
        let single: IceServer =
            serde_json::from_str(r#"{"urls":"stun:stun.example.org"}"#).expect("string urls");
        assert_eq!(single.urls, vec!["stun:stun.example.org"]);
        let many: IceServer = serde_json::from_str(
            r#"{"urls":["turn:t.example.org:3478?transport=udp","turns:t.example.org:443?transport=tcp"],"username":"u","credential":"c"}"#,
        )
        .expect("list urls");
        many.validate("ice").expect("valid TURN entry");
        assert!(many.is_turn());
    }

    #[test]
    fn rejects_turn_without_credentials_and_unknown_schemes() {
        assert!(IceServer::stun("turn:t.example.org")
            .validate("ice")
            .is_err());
        assert!(IceServer::stun("http://t.example.org")
            .validate("ice")
            .is_err());
        assert!(IceServer::stun("stun:").validate("ice").is_err());
        assert!(IceServer::stun("stun:host:0").validate("ice").is_err());
        assert!(IceServer::stun("stun:host?transport=udp")
            .validate("ice")
            .is_err());
        assert!(turn("turn:host?transport=sctp").validate("ice").is_err());
    }

    #[test]
    fn parses_ipv6_hosts_and_reports_lowercase_hosts() {
        let server = turn("turn:[2001:db8::1]:3478");
        server.validate("ice").expect("ipv6 host");
        let upper = turn("TURN:Turn.Example.ORG:3478");
        assert_eq!(upper.hosts().collect::<Vec<_>>(), vec!["turn.example.org"]);
    }

    #[test]
    fn without_port_drops_only_matching_urls() {
        let server = IceServer {
            urls: vec![
                "turn:turn.cloudflare.com:3478?transport=udp".to_string(),
                "turn:turn.cloudflare.com:53?transport=udp".to_string(),
            ],
            username: Some("u".to_string()),
            credential: Some("c".to_string()),
        };
        let kept = server.without_port(53).expect("one url left");
        assert_eq!(kept.urls.len(), 1);
        assert!(turn("turn:host:53").without_port(53).is_none());
    }

    #[test]
    fn debug_never_prints_credentials() {
        let rendered = format!("{:?}", turn("turn:host"));
        assert!(!rendered.contains("\"c\""));
        assert!(rendered.contains("<redacted>"));
    }
}
