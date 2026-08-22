use crate::common::error::config_error;
use std::collections::{HashMap, VecDeque};

use libp2p::PeerId;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::common::error::NetError;
use crate::common::utils::unix_timestamp_ns;

pub const HEARTBEAT_SCHEMA_VERSION: u16 = 2;
pub const HEARTBEAT_ENTROPY_BYTES: usize = 32;
pub const MAX_HEARTBEAT_WIRE_BYTES: usize = 4096;
pub const MAX_HEARTBEAT_AGE_SECS: u64 = 10 * 60;
pub const MAX_HEARTBEAT_FUTURE_SKEW_SECS: u64 = 2 * 60;
pub const DEFAULT_REPLAY_CACHE_CAPACITY: usize = 8192;
pub const DEFAULT_REPLAY_CACHE_TTL_SECS: u64 = 15 * 60;
pub const DEFAULT_MAX_APP_MESSAGE_AGE_SECS: u64 = 10 * 60;
pub const DEFAULT_MAX_APP_MESSAGE_FUTURE_SKEW_SECS: u64 = 2 * 60;
pub const DEFAULT_APP_REPLAY_CACHE_CAPACITY: usize = 8192;
pub const DEFAULT_APP_REPLAY_CACHE_TTL_SECS: u64 = 15 * 60;

const NS_PER_SEC: u64 = 1_000_000_000;
const HEARTBEAT_WIRE_MAGIC: [u8; 4] = *b"P2PH";
const NONCE_BYTES: usize = 32;
const FIXED_WIRE_BYTES: usize =
    HEARTBEAT_WIRE_MAGIC.len() + 2 + 2 + 8 + NONCE_BYTES + HEARTBEAT_ENTROPY_BYTES;

pub fn heartbeat_topic(network_id: u32) -> String {
    format!("p2p-net/heartbeat/v{HEARTBEAT_SCHEMA_VERSION}/net-{network_id}")
}

/// Logical heartbeat envelope. The network wire format is compact binary; serde
/// support remains for diagnostics and embedding APIs and is not used on gossip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatEnvelope {
    /// Topic-specific wire schema version. Missing or wrong versions are rejected.
    #[serde(default)]
    pub schema_version: u16,
    pub peer_id: String,
    pub timestamp_ns: u64,
    pub nonce_hex: String,
    pub entropy: Vec<u8>,
}

impl HeartbeatEnvelope {
    #[must_use]
    pub fn new(peer_id: PeerId) -> Self {
        let mut entropy = vec![0u8; HEARTBEAT_ENTROPY_BYTES];
        OsRng.fill_bytes(&mut entropy);
        let nonce = blake3::hash(&entropy);
        Self {
            schema_version: HEARTBEAT_SCHEMA_VERSION,
            peer_id: peer_id.to_string(),
            timestamp_ns: unix_timestamp_ns(),
            nonce_hex: nonce.to_hex().to_string(),
            entropy,
        }
    }
}

pub fn collect_local_heartbeat(peer_id: PeerId) -> Result<HeartbeatEnvelope, NetError> {
    Ok(HeartbeatEnvelope::new(peer_id))
}

/// Encode the v2 heartbeat as a compact binary frame:
/// magic | schema(u16) | peer_len(u16) | peer_id bytes | timestamp(u64) |
/// nonce([u8;32]) | entropy([u8;32]).
pub fn encode_heartbeat_wire(env: &HeartbeatEnvelope) -> Result<Vec<u8>, NetError> {
    let peer = env
        .peer_id
        .parse::<PeerId>()
        .map_err(|err| NetError::GossipCodec(format!("invalid heartbeat peer id: {err}")))?;
    let peer_bytes = peer.to_bytes();
    let peer_len = u16::try_from(peer_bytes.len())
        .map_err(|_| NetError::GossipCodec("heartbeat peer id is too long".to_string()))?;
    let nonce = decode_nonce(&env.nonce_hex)
        .ok_or_else(|| NetError::GossipCodec("heartbeat nonce must be 32-byte hex".to_string()))?;
    if env.entropy.len() != HEARTBEAT_ENTROPY_BYTES {
        return Err(NetError::GossipCodec(format!(
            "heartbeat entropy must be {HEARTBEAT_ENTROPY_BYTES} bytes"
        )));
    }

    let mut wire = Vec::with_capacity(FIXED_WIRE_BYTES + peer_bytes.len());
    wire.extend_from_slice(&HEARTBEAT_WIRE_MAGIC);
    wire.extend_from_slice(&env.schema_version.to_be_bytes());
    wire.extend_from_slice(&peer_len.to_be_bytes());
    wire.extend_from_slice(&peer_bytes);
    wire.extend_from_slice(&env.timestamp_ns.to_be_bytes());
    wire.extend_from_slice(&nonce);
    wire.extend_from_slice(&env.entropy);
    Ok(wire)
}

fn decode_heartbeat_wire(data: &[u8]) -> Result<HeartbeatEnvelope, NetError> {
    if data.len() < FIXED_WIRE_BYTES {
        return Err(NetError::GossipCodec(
            "heartbeat frame is truncated".to_string(),
        ));
    }
    if data[..HEARTBEAT_WIRE_MAGIC.len()] != HEARTBEAT_WIRE_MAGIC {
        return Err(NetError::GossipCodec(
            "heartbeat frame magic mismatch".to_string(),
        ));
    }

    let mut cursor = HEARTBEAT_WIRE_MAGIC.len();
    let schema_version = read_u16(data, &mut cursor)?;
    let peer_len = usize::from(read_u16(data, &mut cursor)?);
    let expected_len = FIXED_WIRE_BYTES
        .checked_add(peer_len)
        .ok_or_else(|| NetError::GossipCodec("heartbeat frame length overflow".to_string()))?;
    if data.len() != expected_len {
        return Err(NetError::GossipCodec(
            "heartbeat frame length mismatch".to_string(),
        ));
    }

    let peer_end = cursor + peer_len;
    let peer = PeerId::from_bytes(&data[cursor..peer_end])
        .map_err(|err| NetError::GossipCodec(format!("invalid heartbeat peer id bytes: {err}")))?;
    cursor = peer_end;
    let timestamp_ns = read_u64(data, &mut cursor)?;

    let mut nonce = [0u8; NONCE_BYTES];
    nonce.copy_from_slice(&data[cursor..cursor + NONCE_BYTES]);
    cursor += NONCE_BYTES;
    let entropy = data[cursor..cursor + HEARTBEAT_ENTROPY_BYTES].to_vec();

    Ok(HeartbeatEnvelope {
        schema_version,
        peer_id: peer.to_string(),
        timestamp_ns,
        nonce_hex: hex::encode(nonce),
        entropy,
    })
}

fn read_u16(data: &[u8], cursor: &mut usize) -> Result<u16, NetError> {
    let end = cursor.saturating_add(2);
    let bytes: [u8; 2] = data
        .get(*cursor..end)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| NetError::GossipCodec("heartbeat frame is truncated".to_string()))?;
    *cursor = end;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u64(data: &[u8], cursor: &mut usize) -> Result<u64, NetError> {
    let end = cursor.saturating_add(8);
    let bytes: [u8; 8] = data
        .get(*cursor..end)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| NetError::GossipCodec("heartbeat frame is truncated".to_string()))?;
    *cursor = end;
    Ok(u64::from_be_bytes(bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReputationConfig {
    pub accept_reward: i32,
    pub invalid_penalty: i32,
    pub decay_step: i32,
    pub decay_interval_secs: u64,
}

impl Default for ReputationConfig {
    fn default() -> Self {
        Self {
            accept_reward: 1,
            invalid_penalty: 5,
            decay_step: 1,
            decay_interval_secs: 60,
        }
    }
}

impl ReputationConfig {
    pub fn validate(&self) -> Result<(), NetError> {
        if self.accept_reward <= 0 {
            return Err(config_error(
                "message_security.reputation.accept_reward must be positive",
            ));
        }
        if self.invalid_penalty <= 0 {
            return Err(config_error(
                "message_security.reputation.invalid_penalty must be positive",
            ));
        }
        if self.decay_step <= 0 {
            return Err(config_error(
                "message_security.reputation.decay_step must be positive",
            ));
        }
        if self.decay_interval_secs == 0 {
            return Err(config_error(
                "message_security.reputation.decay_interval_secs must be at least 1",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageSecurityConfig {
    pub max_heartbeat_wire_bytes: usize,
    pub max_heartbeat_age_secs: u64,
    pub max_heartbeat_future_skew_secs: u64,
    pub replay_cache_capacity: usize,
    pub replay_cache_ttl_secs: u64,
    pub max_app_message_age_secs: u64,
    pub max_app_message_future_skew_secs: u64,
    pub app_replay_cache_capacity: usize,
    pub app_replay_cache_ttl_secs: u64,
    pub reputation: ReputationConfig,
}

impl Default for MessageSecurityConfig {
    fn default() -> Self {
        Self {
            max_heartbeat_wire_bytes: MAX_HEARTBEAT_WIRE_BYTES,
            max_heartbeat_age_secs: MAX_HEARTBEAT_AGE_SECS,
            max_heartbeat_future_skew_secs: MAX_HEARTBEAT_FUTURE_SKEW_SECS,
            replay_cache_capacity: DEFAULT_REPLAY_CACHE_CAPACITY,
            replay_cache_ttl_secs: DEFAULT_REPLAY_CACHE_TTL_SECS,
            max_app_message_age_secs: DEFAULT_MAX_APP_MESSAGE_AGE_SECS,
            max_app_message_future_skew_secs: DEFAULT_MAX_APP_MESSAGE_FUTURE_SKEW_SECS,
            app_replay_cache_capacity: DEFAULT_APP_REPLAY_CACHE_CAPACITY,
            app_replay_cache_ttl_secs: DEFAULT_APP_REPLAY_CACHE_TTL_SECS,
            reputation: ReputationConfig::default(),
        }
    }
}

impl MessageSecurityConfig {
    pub fn validate(&self) -> Result<(), NetError> {
        if self.max_heartbeat_wire_bytes == 0 {
            return Err(config_error(
                "message_security.max_heartbeat_wire_bytes must be at least 1",
            ));
        }
        if self.max_heartbeat_wire_bytes > 1024 * 1024 {
            return Err(config_error(
                "message_security.max_heartbeat_wire_bytes must not exceed 1048576",
            ));
        }
        if self.max_heartbeat_age_secs == 0 {
            return Err(config_error(
                "message_security.max_heartbeat_age_secs must be at least 1",
            ));
        }
        if self.max_heartbeat_future_skew_secs > self.max_heartbeat_age_secs {
            return Err(config_error(
                "message_security.max_heartbeat_future_skew_secs must be <= max_heartbeat_age_secs",
            ));
        }
        if self.replay_cache_capacity == 0 {
            return Err(config_error(
                "message_security.replay_cache_capacity must be at least 1",
            ));
        }
        if self.replay_cache_ttl_secs == 0 {
            return Err(config_error(
                "message_security.replay_cache_ttl_secs must be at least 1",
            ));
        }
        if self.replay_cache_ttl_secs
            < self
                .max_heartbeat_age_secs
                .saturating_add(self.max_heartbeat_future_skew_secs)
        {
            return Err(config_error(
                "message_security.replay_cache_ttl_secs must cover max_heartbeat_age_secs + max_heartbeat_future_skew_secs",
            ));
        }
        if self.max_app_message_age_secs == 0 {
            return Err(config_error(
                "message_security.max_app_message_age_secs must be at least 1",
            ));
        }
        if self.max_app_message_future_skew_secs > self.max_app_message_age_secs {
            return Err(config_error(
                "message_security.max_app_message_future_skew_secs must be <= max_app_message_age_secs",
            ));
        }
        if self.app_replay_cache_capacity == 0 {
            return Err(config_error(
                "message_security.app_replay_cache_capacity must be at least 1",
            ));
        }
        if self.app_replay_cache_ttl_secs == 0 {
            return Err(config_error(
                "message_security.app_replay_cache_ttl_secs must be at least 1",
            ));
        }
        if self.app_replay_cache_ttl_secs
            < self
                .max_app_message_age_secs
                .saturating_add(self.max_app_message_future_skew_secs)
        {
            return Err(config_error(
                "message_security.app_replay_cache_ttl_secs must cover max_app_message_age_secs + max_app_message_future_skew_secs",
            ));
        }
        self.reputation.validate()
    }
}

#[must_use]
pub fn verify_heartbeat(source: PeerId, env: &HeartbeatEnvelope, now_ns: u64) -> bool {
    verify_heartbeat_with_config(source, env, now_ns, &MessageSecurityConfig::default())
}

#[must_use]
pub fn verify_heartbeat_with_config(
    source: PeerId,
    env: &HeartbeatEnvelope,
    now_ns: u64,
    cfg: &MessageSecurityConfig,
) -> bool {
    if env.schema_version != HEARTBEAT_SCHEMA_VERSION {
        return false;
    }
    if env.peer_id != source.to_string() {
        return false;
    }
    if env.entropy.len() != HEARTBEAT_ENTROPY_BYTES {
        return false;
    }
    if env.entropy.iter().all(|b| *b == 0) {
        return false;
    }
    let Some(nonce) = decode_nonce(&env.nonce_hex) else {
        return false;
    };
    if blake3::hash(&env.entropy).as_bytes() != nonce.as_slice() {
        return false;
    }

    let max_future_ns = cfg
        .max_heartbeat_future_skew_secs
        .saturating_mul(NS_PER_SEC);
    let max_age_ns = cfg.max_heartbeat_age_secs.saturating_mul(NS_PER_SEC);
    if env.timestamp_ns > now_ns.saturating_add(max_future_ns) {
        return false;
    }
    if now_ns.saturating_sub(env.timestamp_ns) > max_age_ns {
        return false;
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatValidationDecision {
    Accept,
    Reject,
    IgnoreDuplicate,
    RejectOversize,
}

#[derive(Debug, Clone)]
pub struct HeartbeatValidationResult {
    pub decision: HeartbeatValidationDecision,
    pub envelope: Option<HeartbeatEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HeartbeatReplayKey {
    source: PeerId,
    schema_version: u16,
    timestamp_ns: u64,
    nonce: [u8; NONCE_BYTES],
}

#[derive(Debug, Clone)]
pub struct HeartbeatReplayCache {
    capacity: usize,
    ttl_ns: u64,
    entries: HashMap<HeartbeatReplayKey, u64>,
    order: VecDeque<HeartbeatReplayKey>,
}

impl HeartbeatReplayCache {
    #[must_use]
    pub fn new(cfg: &MessageSecurityConfig) -> Self {
        Self {
            capacity: cfg.replay_cache_capacity.max(1),
            ttl_ns: cfg.replay_cache_ttl_secs.saturating_mul(NS_PER_SEC),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns `true` when the heartbeat is new, or `false` when it is a replay.
    pub fn check_and_insert(
        &mut self,
        source: PeerId,
        env: &HeartbeatEnvelope,
        now_ns: u64,
    ) -> bool {
        self.prune(now_ns);
        let Some(key) = replay_key(source, env) else {
            return false;
        };
        if self.entries.contains_key(&key) {
            return false;
        }
        self.entries.insert(key.clone(), now_ns);
        self.order.push_back(key);
        while self.entries.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }
        true
    }

    fn prune(&mut self, now_ns: u64) {
        while let Some(front) = self.order.front() {
            let Some(inserted_ns) = self.entries.get(front).copied() else {
                let _ = self.order.pop_front();
                continue;
            };
            if now_ns.saturating_sub(inserted_ns) <= self.ttl_ns {
                break;
            }
            let Some(expired) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&expired);
        }
    }
}

pub fn validate_heartbeat_wire(
    source: PeerId,
    data: &[u8],
    now_ns: u64,
    cfg: &MessageSecurityConfig,
    replay_cache: &mut HeartbeatReplayCache,
) -> HeartbeatValidationResult {
    if data.len() > cfg.max_heartbeat_wire_bytes {
        return HeartbeatValidationResult {
            decision: HeartbeatValidationDecision::RejectOversize,
            envelope: None,
        };
    }

    let Ok(env) = decode_heartbeat_wire(data) else {
        return HeartbeatValidationResult {
            decision: HeartbeatValidationDecision::Reject,
            envelope: None,
        };
    };

    if !verify_heartbeat_with_config(source, &env, now_ns, cfg) {
        return HeartbeatValidationResult {
            decision: HeartbeatValidationDecision::Reject,
            envelope: Some(env),
        };
    }

    if !replay_cache.check_and_insert(source, &env, now_ns) {
        return HeartbeatValidationResult {
            decision: HeartbeatValidationDecision::IgnoreDuplicate,
            envelope: Some(env),
        };
    }

    HeartbeatValidationResult {
        decision: HeartbeatValidationDecision::Accept,
        envelope: Some(env),
    }
}

fn replay_key(source: PeerId, env: &HeartbeatEnvelope) -> Option<HeartbeatReplayKey> {
    Some(HeartbeatReplayKey {
        source,
        schema_version: env.schema_version,
        timestamp_ns: env.timestamp_ns,
        nonce: decode_nonce(&env.nonce_hex)?,
    })
}

fn decode_nonce(nonce_hex: &str) -> Option<[u8; NONCE_BYTES]> {
    let decoded = hex::decode(nonce_hex).ok()?;
    decoded.try_into().ok()
}
