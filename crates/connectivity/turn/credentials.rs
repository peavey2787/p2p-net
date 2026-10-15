//! The provider-agnostic TURN credential bundle.
//!
//! This one JSON shape is the HTTP provider contract (what a Cloudflare
//! Worker or any other credential endpoint returns) and the libp2p broker
//! response, so every provider plugs into the same pipeline:
//!
//! ```json
//! { "iceServers": [ { "urls": ["turn:turn.example.org:3478"], "username": "u", "credential": "c" } ],
//!   "ttl": 3600 }
//! ```

use serde::{Deserialize, Serialize};

use crate::connectivity::ice::{one_or_many, IceServer};

/// Most entries kept from any single credential source.
pub const MAX_ICE_SERVERS_PER_SOURCE: usize = 8;
/// Credentials without a `ttl` are refreshed as if they lived this long.
pub const DEFAULT_CREDENTIAL_TTL_SECS: u64 = 3600;
/// Shortest and longest lifetimes accepted for issued credentials.
pub const MIN_CREDENTIAL_TTL_SECS: u64 = 60;
pub const MAX_CREDENTIAL_TTL_SECS: u64 = 48 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCredentials {
    /// One object (Cloudflare's older endpoint) or a list.
    #[serde(rename = "iceServers", deserialize_with = "one_or_many")]
    pub ice_servers: Vec<IceServer>,
    /// Lifetime in seconds from issue time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u64>,
}

impl TurnCredentials {
    /// Keep only valid entries (at most [`MAX_ICE_SERVERS_PER_SOURCE`]) and
    /// clamp the lifetime. Returns the number of entries dropped.
    pub fn sanitize(&mut self) -> usize {
        let before = self.ice_servers.len();
        self.ice_servers
            .retain(|server| server.validate("iceServers").is_ok());
        self.ice_servers.truncate(MAX_ICE_SERVERS_PER_SOURCE);
        self.ttl = Some(self.ttl_secs());
        before - self.ice_servers.len()
    }

    /// Effective lifetime, clamped to the accepted range.
    pub fn ttl_secs(&self) -> u64 {
        self.ttl
            .unwrap_or(DEFAULT_CREDENTIAL_TTL_SECS)
            .clamp(MIN_CREDENTIAL_TTL_SECS, MAX_CREDENTIAL_TTL_SECS)
    }

    pub fn has_turn(&self) -> bool {
        self.ice_servers.iter().any(IceServer::is_turn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_object_or_list_and_sanitizes() {
        let mut single: TurnCredentials = serde_json::from_str(
            r#"{"iceServers":{"urls":["turn:t.example.org"],"username":"u","credential":"c"}}"#,
        )
        .expect("object form");
        assert_eq!(single.sanitize(), 0);
        assert_eq!(single.ttl, Some(DEFAULT_CREDENTIAL_TTL_SECS));
        assert!(single.has_turn());

        let mut list: TurnCredentials = serde_json::from_str(
            r#"{"iceServers":[{"urls":"turn:no-creds.example.org"},{"urls":"stun:s.example.org"}],"ttl":5}"#,
        )
        .expect("list form");
        assert_eq!(list.sanitize(), 1);
        assert_eq!(list.ttl, Some(MIN_CREDENTIAL_TTL_SECS));
        assert!(!list.has_turn());
    }

    #[test]
    fn caps_entries_per_source() {
        let mut creds = TurnCredentials {
            ice_servers: (0..20)
                .map(|i| IceServer::stun(format!("stun:s{i}.example.org")))
                .collect(),
            ttl: Some(MAX_CREDENTIAL_TTL_SECS * 2),
        };
        creds.sanitize();
        assert_eq!(creds.ice_servers.len(), MAX_ICE_SERVERS_PER_SOURCE);
        assert_eq!(creds.ttl_secs(), MAX_CREDENTIAL_TTL_SECS);
    }
}
