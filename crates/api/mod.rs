//! Stable application-facing primitives for the decentralized networking core.
//!
//! The public API intentionally stays small: connect peers, disconnect peers,
//! send addressed messages, broadcast to a topic, subscribe to topics, and read
//! known peers. Higher-level systems such as chat, games, storage, compute, and
//! pub/sub apps should build on these primitives instead of depending on
//! libp2p-specific swarm internals.

use libp2p::{gossipsub::IdentTopic, PeerId};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::common::error::NetError;
use crate::common::utils::unix_timestamp_ns;

pub const APP_MESSAGE_SCHEMA_VERSION: u16 = 1;
pub const APP_TOPIC_PREFIX: &str = "p2p-net/app";
pub const MAX_APP_TOPIC_LEN: usize = 128;
pub const MAX_APP_MESSAGE_BYTES: usize = 1024 * 1024;

/// How the local node learned about a peer.
///
/// These sources let applications distinguish directly connected peers from
/// peers learned through bootstrap, rendezvous, relay discovery, DHT provider
/// lookup, or local cache. The public primitive remains `get_peers`; this enum
/// makes the returned metadata useful for connection planning without exposing
/// libp2p internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSource {
    Connected,
    Bootstrap,
    BootstrapSeed,
    PublicBootstrapSeed,
    Rendezvous,
    PublicRendezvous,
    DhtProvider,
    RelayDiscovery,
    PublicRelayDiscovery,
    PeerCache,
    Manual,
}

impl PeerSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Bootstrap => "bootstrap",
            Self::BootstrapSeed => "bootstrap_seed",
            Self::PublicBootstrapSeed => "public_bootstrap_seed",
            Self::Rendezvous => "rendezvous",
            Self::PublicRendezvous => "public_rendezvous",
            Self::DhtProvider => "dht_provider",
            Self::RelayDiscovery => "relay_discovery",
            Self::PublicRelayDiscovery => "public_relay_discovery",
            Self::PeerCache => "peer_cache",
            Self::Manual => "manual",
        }
    }
}

/// A peer visible through the app-facing API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub connected: bool,
    pub addresses: Vec<String>,
    #[serde(default)]
    pub sources: Vec<PeerSource>,
    #[serde(default)]
    pub supports_relay: Option<bool>,
    #[serde(default)]
    pub supports_rendezvous: Option<bool>,
    #[serde(default)]
    pub supports_dcutr: Option<bool>,
    #[serde(default)]
    pub last_seen_unix_secs: Option<u64>,
    #[serde(default)]
    pub namespace: Option<String>,
}

impl PeerInfo {
    #[must_use]
    pub fn connected(peer_id: PeerId) -> Self {
        Self {
            peer_id: peer_id.to_string(),
            connected: true,
            addresses: Vec::new(),
            sources: vec![PeerSource::Connected],
            supports_relay: None,
            supports_rendezvous: None,
            supports_dcutr: None,
            last_seen_unix_secs: None,
            namespace: None,
        }
    }

    #[must_use]
    pub fn discovered(
        peer_id: PeerId,
        source: PeerSource,
        addresses: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            peer_id: peer_id.to_string(),
            connected: false,
            addresses: addresses.into_iter().collect(),
            sources: vec![source],
            supports_relay: None,
            supports_rendezvous: None,
            supports_dcutr: None,
            last_seen_unix_secs: None,
            namespace: None,
        }
    }

    #[must_use]
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    #[must_use]
    pub fn has_source(&self, source: PeerSource) -> bool {
        self.sources.contains(&source)
    }
}

/// Topic-filtered local subscription returned by `NodeHandle::subscribe`.
pub struct AppSubscription {
    topic: String,
    receiver: broadcast::Receiver<AppMessage>,
}

impl AppSubscription {
    #[must_use]
    pub fn new(topic: String, receiver: broadcast::Receiver<AppMessage>) -> Self {
        Self { topic, receiver }
    }

    #[must_use]
    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub async fn recv(&mut self) -> Result<AppMessage, broadcast::error::RecvError> {
        loop {
            let message = self.receiver.recv().await?;
            if message.topic == self.topic {
                return Ok(message);
            }
        }
    }
}

/// Application payload envelope carried by the shared P2P core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppMessage {
    #[serde(default = "default_app_message_schema_version")]
    pub schema_version: u16,
    pub network_id: u32,
    pub topic: String,
    pub source_peer_id: String,
    pub target_peer_id: Option<String>,
    pub timestamp_ns: u64,
    pub nonce_hex: String,
    pub payload: Vec<u8>,
}

impl AppMessage {
    pub fn addressed(
        network_id: u32,
        topic: impl Into<String>,
        source_peer_id: PeerId,
        target_peer_id: PeerId,
        payload: Vec<u8>,
    ) -> Result<Self, NetError> {
        Self::new(
            network_id,
            topic,
            source_peer_id,
            Some(target_peer_id),
            payload,
        )
    }

    pub fn broadcast(
        network_id: u32,
        topic: impl Into<String>,
        source_peer_id: PeerId,
        payload: Vec<u8>,
    ) -> Result<Self, NetError> {
        Self::new(network_id, topic, source_peer_id, None, payload)
    }

    fn new(
        network_id: u32,
        topic: impl Into<String>,
        source_peer_id: PeerId,
        target_peer_id: Option<PeerId>,
        payload: Vec<u8>,
    ) -> Result<Self, NetError> {
        let topic = normalize_app_topic(topic.into())?;
        validate_app_payload_len(payload.len())?;
        let mut entropy = [0u8; 32];
        OsRng.fill_bytes(&mut entropy);
        let nonce_hex = blake3::hash(&entropy).to_hex().to_string();
        Ok(Self {
            schema_version: APP_MESSAGE_SCHEMA_VERSION,
            network_id,
            topic,
            source_peer_id: source_peer_id.to_string(),
            target_peer_id: target_peer_id.map(|peer| peer.to_string()),
            timestamp_ns: unix_timestamp_ns(),
            nonce_hex,
            payload,
        })
    }

    #[must_use]
    pub fn is_for_peer(&self, local_peer: &PeerId) -> bool {
        let local_peer = local_peer.to_string();
        match self.target_peer_id.as_deref() {
            Some(target) => target == local_peer.as_str(),
            None => true,
        }
    }
}

pub fn app_topic_name(network_id: u32, topic: impl AsRef<str>) -> Result<String, NetError> {
    let topic = normalize_app_topic(topic.as_ref())?;
    Ok(format!(
        "{APP_TOPIC_PREFIX}/v{APP_MESSAGE_SCHEMA_VERSION}/net-{network_id}/{topic}"
    ))
}

pub fn app_ident_topic(network_id: u32, topic: impl AsRef<str>) -> Result<IdentTopic, NetError> {
    Ok(IdentTopic::new(app_topic_name(network_id, topic)?))
}

pub fn encode_app_message(message: &AppMessage) -> Result<Vec<u8>, NetError> {
    let encoded = serde_json::to_vec(message).map_err(|err| NetError::AppMessage {
        topic: message.topic.clone(),
        reason: err.to_string(),
    })?;
    validate_app_payload_len(encoded.len())?;
    Ok(encoded)
}

pub fn decode_app_message(raw: &[u8]) -> Result<AppMessage, NetError> {
    validate_app_payload_len(raw.len())?;
    let message: AppMessage = serde_json::from_slice(raw).map_err(|err| NetError::AppMessage {
        topic: "<unknown>".to_string(),
        reason: err.to_string(),
    })?;
    validate_app_message(&message)?;
    Ok(message)
}

pub fn validate_app_message(message: &AppMessage) -> Result<(), NetError> {
    if message.schema_version != APP_MESSAGE_SCHEMA_VERSION {
        return Err(NetError::AppMessage {
            topic: message.topic.clone(),
            reason: format!(
                "unsupported schema version {}; expected {APP_MESSAGE_SCHEMA_VERSION}",
                message.schema_version
            ),
        });
    }
    normalize_app_topic(&message.topic)?;
    message
        .source_peer_id
        .parse::<PeerId>()
        .map_err(|err| NetError::AppMessage {
            topic: message.topic.clone(),
            reason: format!("invalid source peer id: {err}"),
        })?;
    if let Some(target) = &message.target_peer_id {
        target
            .parse::<PeerId>()
            .map_err(|err| NetError::AppMessage {
                topic: message.topic.clone(),
                reason: format!("invalid target peer id: {err}"),
            })?;
    }
    validate_app_payload_len(message.payload.len())
}

pub fn normalize_app_topic(topic: impl AsRef<str>) -> Result<String, NetError> {
    let topic = topic.as_ref().trim();
    if topic.is_empty() {
        return Err(NetError::AppTopic {
            topic: topic.to_string(),
            reason: "topic must not be empty".to_string(),
        });
    }
    if topic.len() > MAX_APP_TOPIC_LEN {
        return Err(NetError::AppTopic {
            topic: topic.to_string(),
            reason: format!("topic must not exceed {MAX_APP_TOPIC_LEN} bytes"),
        });
    }
    if !topic
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(NetError::AppTopic {
            topic: topic.to_string(),
            reason: "topic may only contain ASCII letters, numbers, '-', '_', '.', or '/'"
                .to_string(),
        });
    }
    Ok(topic.to_string())
}

fn validate_app_payload_len(len: usize) -> Result<(), NetError> {
    if len > MAX_APP_MESSAGE_BYTES {
        return Err(NetError::AppMessage {
            topic: "<unknown>".to_string(),
            reason: format!("message must not exceed {MAX_APP_MESSAGE_BYTES} bytes"),
        });
    }
    Ok(())
}

fn default_app_message_schema_version() -> u16 {
    APP_MESSAGE_SCHEMA_VERSION
}
