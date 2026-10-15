//! TURN credential broker over libp2p (`/p2p-net/turn-credentials/1.0.0`).
//!
//! A broker node keeps fresh credentials from its sources (the built-in TURN
//! server and/or an upstream provider such as Cloudflare) and answers peer
//! requests from that cache, so long-lived secrets never leave the server and
//! browsers need no configuration at all. Requests travel over the
//! authenticated, encrypted libp2p connection and are rate limited per peer.

use std::collections::HashMap;
use std::time::Duration;

use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use web_time::Instant;

use super::client::LearnedServers;
use super::config::TurnBrokerConfig;
use super::credentials::TurnCredentials;
use super::provider::TurnProviderConfig;
use super::refresh::ProviderRefresher;

pub const TURN_CREDENTIALS_PROTOCOL: &str = "/p2p-net/turn-credentials/1.0.0";
/// Encoded request/response size caps for the JSON codec.
pub const MAX_REQUEST_BYTES: u64 = 1024;
pub const MAX_RESPONSE_BYTES: u64 = 16 * 1024;

const RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_RATE_LIMITED_PEERS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRequest {
    pub ttl_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialResponse {
    Granted(TurnCredentials),
    /// No source currently has credentials; ask again later.
    Unavailable,
    RateLimited,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnBrokerStatus {
    pub sources: Vec<&'static str>,
    pub ready_sources: usize,
    pub granted: usize,
    pub rate_limited: usize,
    pub unavailable: usize,
    pub source_errors: usize,
    pub last_error: Option<String>,
}

struct BrokerSource {
    refresher: ProviderRefresher,
    cached: Option<LearnedServers>,
}

pub struct TurnBrokerState {
    sources: Vec<BrokerSource>,
    ttl_secs: u64,
    max_per_window: u32,
    windows: HashMap<PeerId, (Instant, u32)>,
    status: TurnBrokerStatus,
}

impl TurnBrokerState {
    pub fn new(cfg: &TurnBrokerConfig) -> Self {
        let mut state = Self {
            sources: Vec::new(),
            ttl_secs: cfg.credential_ttl_secs,
            max_per_window: cfg.max_requests_per_peer_per_minute,
            windows: HashMap::new(),
            status: TurnBrokerStatus::default(),
        };
        if let Some(provider) = &cfg.provider {
            state.add_source(provider.clone());
        }
        state
    }

    /// Add a credential source (e.g. the built-in TURN server once it runs).
    pub fn add_source(&mut self, provider: TurnProviderConfig) {
        self.status.sources.push(provider.name());
        self.sources.push(BrokerSource {
            refresher: ProviderRefresher::new(provider, self.ttl_secs),
            cached: None,
        });
    }

    /// Start due refreshes and collect finished ones. Returns pulse lines.
    pub fn maintain(&mut self, now: Instant) -> Vec<String> {
        let mut pulses = Vec::new();
        for source in &mut self.sources {
            if let Some(result) = source.refresher.take_result(now) {
                let name = source.refresher.provider_name();
                match result {
                    Ok(credentials) => {
                        pulses.push(format!(
                            "TURN_BROKER_REFRESHED source={name} servers={} ttl={}",
                            credentials.ice_servers.len(),
                            credentials.ttl_secs()
                        ));
                        source.cached = Some(LearnedServers::new(credentials, now));
                    }
                    Err(error) => {
                        self.status.source_errors = self.status.source_errors.saturating_add(1);
                        pulses.push(format!(
                            "TURN_BROKER_SOURCE_FAILED source={name} error={error}"
                        ));
                        self.status.last_error = Some(error);
                    }
                }
            }
            source.refresher.start_if_due(now);
            if source.cached.as_ref().is_some_and(|c| c.expires_at <= now) {
                source.cached = None;
            }
        }
        self.status.ready_sources = self.sources.iter().filter(|s| s.cached.is_some()).count();
        pulses
    }

    /// Answer one peer request from the cache.
    pub fn answer(&mut self, peer: PeerId, now: Instant) -> CredentialResponse {
        if !self.admit(peer, now) {
            self.status.rate_limited = self.status.rate_limited.saturating_add(1);
            return CredentialResponse::RateLimited;
        }
        let live: Vec<&LearnedServers> = self
            .sources
            .iter()
            .filter_map(|source| source.cached.as_ref())
            .filter(|cached| cached.expires_at > now)
            .collect();
        let Some(remaining) = live
            .iter()
            .map(|cached| cached.expires_at.saturating_duration_since(now).as_secs())
            .min()
        else {
            self.status.unavailable = self.status.unavailable.saturating_add(1);
            return CredentialResponse::Unavailable;
        };
        let ice_servers = live
            .iter()
            .flat_map(|cached| cached.servers.iter().cloned())
            .collect();
        self.status.granted = self.status.granted.saturating_add(1);
        CredentialResponse::Granted(TurnCredentials {
            ice_servers,
            ttl: Some(remaining),
        })
    }

    pub fn status(&self) -> &TurnBrokerStatus {
        &self.status
    }

    fn admit(&mut self, peer: PeerId, now: Instant) -> bool {
        if self.windows.len() >= MAX_RATE_LIMITED_PEERS {
            self.windows
                .retain(|_, (start, _)| now.saturating_duration_since(*start) < RATE_WINDOW);
        }
        let window = self.windows.entry(peer).or_insert((now, 0));
        if now.saturating_duration_since(window.0) >= RATE_WINDOW {
            *window = (now, 0);
        }
        if window.1 >= self.max_per_window {
            return false;
        }
        window.1 += 1;
        true
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::connectivity::turn::{SecretRef, SharedSecretProviderConfig};

    fn broker(max_per_minute: u32) -> TurnBrokerState {
        TurnBrokerState::new(&TurnBrokerConfig {
            enabled: true,
            provider: Some(TurnProviderConfig::SharedSecret(
                SharedSecretProviderConfig {
                    urls: vec!["turn:turn.example.org:3478".to_string()],
                    secret: SecretRef::Value("secret".to_string()),
                },
            )),
            credential_ttl_secs: 600,
            max_requests_per_peer_per_minute: max_per_minute,
        })
    }

    async fn ready(broker: &mut TurnBrokerState) {
        for _ in 0..200 {
            broker.maintain(Instant::now());
            if broker.status().ready_sources == 1 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("broker source never became ready");
    }

    #[tokio::test]
    async fn grants_cached_credentials_once_a_source_is_ready() {
        let mut broker = broker(6);
        let peer = PeerId::random();
        assert_eq!(
            broker.answer(peer, Instant::now()),
            CredentialResponse::Unavailable
        );
        ready(&mut broker).await;
        let CredentialResponse::Granted(credentials) = broker.answer(peer, Instant::now()) else {
            panic!("expected granted credentials");
        };
        assert!(credentials.has_turn());
        assert!(credentials.ttl.is_some_and(|ttl| ttl <= 600 && ttl > 590));
        assert_eq!(broker.status().granted, 1);
    }

    #[tokio::test]
    async fn rate_limits_each_peer_per_window() {
        let mut broker = broker(2);
        ready(&mut broker).await;
        let peer = PeerId::random();
        let now = Instant::now();
        assert!(matches!(
            broker.answer(peer, now),
            CredentialResponse::Granted(_)
        ));
        assert!(matches!(
            broker.answer(peer, now),
            CredentialResponse::Granted(_)
        ));
        assert_eq!(broker.answer(peer, now), CredentialResponse::RateLimited);
        assert!(matches!(
            broker.answer(PeerId::random(), now),
            CredentialResponse::Granted(_)
        ));
        assert!(matches!(
            broker.answer(peer, now + RATE_WINDOW),
            CredentialResponse::Granted(_)
        ));
    }

    #[test]
    fn wire_format_is_tagged_json() {
        let json = serde_json::to_string(&CredentialResponse::RateLimited).expect("encode");
        assert_eq!(json, "\"rate_limited\"");
    }
}
