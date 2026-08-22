//! Freshness and replay protection for signed application envelopes.

use std::collections::{HashMap, VecDeque};

use libp2p::PeerId;

use crate::api::AppMessage;

use super::pulse::MessageSecurityConfig;

const NS_PER_SEC: u64 = 1_000_000_000;
const APP_NONCE_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppMessageSecurityDecision {
    Accept,
    Reject,
    IgnoreDuplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AppReplayKey {
    source: PeerId,
    timestamp_ns: u64,
    nonce: [u8; APP_NONCE_BYTES],
}

#[derive(Debug, Clone)]
pub(crate) struct AppMessageReplayCache {
    capacity: usize,
    ttl_ns: u64,
    entries: HashMap<AppReplayKey, u64>,
    order: VecDeque<AppReplayKey>,
}

impl AppMessageReplayCache {
    #[must_use]
    pub(crate) fn new(cfg: &MessageSecurityConfig) -> Self {
        Self {
            capacity: cfg.app_replay_cache_capacity.max(1),
            ttl_ns: cfg.app_replay_cache_ttl_secs.saturating_mul(NS_PER_SEC),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn check_and_insert(&mut self, key: AppReplayKey, now_ns: u64) -> bool {
        self.prune(now_ns);
        if self.entries.contains_key(&key) {
            return false;
        }
        self.entries.insert(key.clone(), now_ns);
        self.order.push_back(key);
        while self.entries.len() > self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
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

pub(crate) fn validate_app_message_security(
    source: PeerId,
    message: &AppMessage,
    now_ns: u64,
    cfg: &MessageSecurityConfig,
    replay_cache: &mut AppMessageReplayCache,
) -> AppMessageSecurityDecision {
    let Some(nonce) = decode_nonce(&message.nonce_hex) else {
        return AppMessageSecurityDecision::Reject;
    };
    if nonce.iter().all(|byte| *byte == 0) {
        return AppMessageSecurityDecision::Reject;
    }

    let max_future_ns = cfg
        .max_app_message_future_skew_secs
        .saturating_mul(NS_PER_SEC);
    let max_age_ns = cfg.max_app_message_age_secs.saturating_mul(NS_PER_SEC);
    if message.timestamp_ns > now_ns.saturating_add(max_future_ns)
        || now_ns.saturating_sub(message.timestamp_ns) > max_age_ns
    {
        return AppMessageSecurityDecision::Reject;
    }

    let key = AppReplayKey {
        source,
        timestamp_ns: message.timestamp_ns,
        nonce,
    };
    if !replay_cache.check_and_insert(key, now_ns) {
        return AppMessageSecurityDecision::IgnoreDuplicate;
    }
    AppMessageSecurityDecision::Accept
}

fn decode_nonce(nonce_hex: &str) -> Option<[u8; APP_NONCE_BYTES]> {
    if nonce_hex.len() != APP_NONCE_BYTES * 2 {
        return None;
    }
    let mut nonce = [0u8; APP_NONCE_BYTES];
    hex::decode_to_slice(nonce_hex, &mut nonce).ok()?;
    Some(nonce)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_stale_future_and_duplicate_application_envelopes() {
        let source = PeerId::random();
        let cfg = MessageSecurityConfig {
            max_app_message_age_secs: 60,
            max_app_message_future_skew_secs: 5,
            ..MessageSecurityConfig::default()
        };
        let now = 1_000 * NS_PER_SEC;

        let mut valid = AppMessage::broadcast(1, "security", source, vec![1]).unwrap();
        valid.timestamp_ns = now;
        let mut cache = AppMessageReplayCache::new(&cfg);
        assert_eq!(
            validate_app_message_security(source, &valid, now, &cfg, &mut cache),
            AppMessageSecurityDecision::Accept
        );
        assert_eq!(
            validate_app_message_security(source, &valid, now, &cfg, &mut cache),
            AppMessageSecurityDecision::IgnoreDuplicate
        );

        let mut stale = AppMessage::broadcast(1, "security", source, vec![2]).unwrap();
        stale.timestamp_ns = now - 61 * NS_PER_SEC;
        assert_eq!(
            validate_app_message_security(source, &stale, now, &cfg, &mut cache),
            AppMessageSecurityDecision::Reject
        );

        let mut future = AppMessage::broadcast(1, "security", source, vec![3]).unwrap();
        future.timestamp_ns = now + 6 * NS_PER_SEC;
        assert_eq!(
            validate_app_message_security(source, &future, now, &cfg, &mut cache),
            AppMessageSecurityDecision::Reject
        );
    }

    #[test]
    fn rejects_malformed_or_zero_nonce() {
        let source = PeerId::random();
        let cfg = MessageSecurityConfig::default();
        let now = 1_000 * NS_PER_SEC;
        let mut cache = AppMessageReplayCache::new(&cfg);
        let mut message = AppMessage::broadcast(1, "security", source, vec![1]).unwrap();
        message.timestamp_ns = now;
        message.nonce_hex = "00".repeat(APP_NONCE_BYTES);
        assert_eq!(
            validate_app_message_security(source, &message, now, &cfg, &mut cache),
            AppMessageSecurityDecision::Reject
        );
        message.nonce_hex = "not-hex".to_string();
        assert_eq!(
            validate_app_message_security(source, &message, now, &cfg, &mut cache),
            AppMessageSecurityDecision::Reject
        );
    }
}
