use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use libp2p::PeerId;

use crate::protocol::pulse::ReputationConfig;

const MAX_TRACKED_REPUTATION_PEERS: usize = 8_192;
const NEUTRAL_RETENTION_INTERVALS: u32 = 10;

#[derive(Debug, Clone)]
pub struct ReputationEntry {
    pub score: i32,
    pub invalid: u64,
    pub accepted: u64,
    pub duplicate_ignored: u64,
    pub last_updated: Instant,
}

impl Default for ReputationEntry {
    fn default() -> Self {
        Self {
            score: 0,
            invalid: 0,
            accepted: 0,
            duplicate_ignored: 0,
            last_updated: Instant::now(),
        }
    }
}

/// Bounded recent peer reputation.
///
/// A public full node can observe an unbounded sequence of peer IDs over its
/// lifetime. Reputation history therefore cannot be an append-only map. The
/// store keeps a bounded oldest-first index and removes neutral, inactive
/// entries after several decay intervals. Negative/positive scores still decay
/// according to `ReputationConfig`; capacity is a final memory-DoS guard.
#[derive(Debug)]
pub struct ReputationStore {
    by_peer: HashMap<PeerId, ReputationEntry>,
    age_order: BTreeSet<(Instant, PeerId)>,
    cfg: ReputationConfig,
    max_peers: usize,
}

impl Default for ReputationStore {
    fn default() -> Self {
        Self::new(ReputationConfig::default())
    }
}

impl ReputationStore {
    #[must_use]
    pub fn new(cfg: ReputationConfig) -> Self {
        Self::with_capacity(cfg, MAX_TRACKED_REPUTATION_PEERS)
    }

    fn with_capacity(cfg: ReputationConfig, max_peers: usize) -> Self {
        Self {
            by_peer: HashMap::new(),
            age_order: BTreeSet::new(),
            cfg,
            max_peers: max_peers.max(1),
        }
    }

    pub fn accept(&mut self, peer: PeerId) {
        let reward = self.cfg.accept_reward;
        self.update_peer(peer, |ent| {
            ent.accepted = ent.accepted.saturating_add(1);
            ent.score = ent.score.saturating_add(reward);
        });
    }

    pub fn penalize_invalid(&mut self, peer: PeerId) {
        let penalty = self.cfg.invalid_penalty;
        self.update_peer(peer, |ent| {
            ent.invalid = ent.invalid.saturating_add(1);
            ent.score = ent.score.saturating_sub(penalty);
        });
    }

    pub fn ignore_duplicate(&mut self, peer: PeerId) {
        self.update_peer(peer, |ent| {
            ent.duplicate_ignored = ent.duplicate_ignored.saturating_add(1);
        });
    }

    /// Decay inactive reputation and discard neutral history after a bounded
    /// retention period. This must be called by runtime maintenance.
    pub fn tick_decay(&mut self) {
        let now = Instant::now();
        let interval = Duration::from_secs(self.cfg.decay_interval_secs.max(1));
        let neutral_retention = interval
            .checked_mul(NEUTRAL_RETENTION_INTERVALS)
            .unwrap_or(Duration::MAX);
        let step = self.cfg.decay_step.max(1);

        let peers = self.by_peer.keys().copied().collect::<Vec<_>>();
        for peer in peers {
            let Some(entry) = self.by_peer.get(&peer) else {
                continue;
            };
            let age = now.duration_since(entry.last_updated);
            if entry.score == 0 && age >= neutral_retention {
                self.remove_peer(&peer);
                continue;
            }
            if entry.score == 0 || age < interval {
                continue;
            }

            let old_updated = entry.last_updated;
            self.age_order.remove(&(old_updated, peer));
            let Some(entry) = self.by_peer.get_mut(&peer) else {
                continue;
            };
            if entry.score < 0 {
                entry.score = entry.score.saturating_add(step).min(0);
            } else {
                entry.score = entry.score.saturating_sub(step).max(0);
            }
            entry.last_updated = now;
            self.age_order.insert((now, peer));
        }
    }

    fn update_peer(&mut self, peer: PeerId, update: impl FnOnce(&mut ReputationEntry)) {
        let now = Instant::now();
        if let Some(entry) = self.by_peer.get_mut(&peer) {
            self.age_order.remove(&(entry.last_updated, peer));
            update(entry);
            entry.last_updated = now;
        } else {
            let mut entry = ReputationEntry {
                last_updated: now,
                ..ReputationEntry::default()
            };
            update(&mut entry);
            self.by_peer.insert(peer, entry);
        }
        self.age_order.insert((now, peer));
        self.enforce_capacity();
    }

    fn enforce_capacity(&mut self) {
        while self.by_peer.len() > self.max_peers {
            let Some(oldest) = self.age_order.iter().next().copied() else {
                break;
            };
            self.age_order.remove(&oldest);
            self.by_peer.remove(&oldest.1);
        }
    }

    fn remove_peer(&mut self, peer: &PeerId) -> bool {
        let Some(entry) = self.by_peer.remove(peer) else {
            return false;
        };
        self.age_order.remove(&(entry.last_updated, *peer));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reputation_store_is_hard_bounded_under_peer_churn() {
        let mut store = ReputationStore::with_capacity(ReputationConfig::default(), 4);
        for _ in 0..32 {
            store.accept(PeerId::random());
        }

        assert_eq!(store.by_peer.len(), 4);
        assert_eq!(store.age_order.len(), 4);
    }

    #[test]
    fn neutral_idle_reputation_is_reclaimed() {
        let cfg = ReputationConfig {
            decay_interval_secs: 1,
            ..ReputationConfig::default()
        };
        let mut store = ReputationStore::with_capacity(cfg, 4);
        let peer = PeerId::random();
        store.ignore_duplicate(peer);
        let old = Instant::now() - Duration::from_secs(u64::from(NEUTRAL_RETENTION_INTERVALS) + 1);
        let entry = store.by_peer.get_mut(&peer).expect("peer reputation");
        store.age_order.remove(&(entry.last_updated, peer));
        entry.last_updated = old;
        store.age_order.insert((old, peer));

        store.tick_decay();

        assert!(!store.by_peer.contains_key(&peer));
        assert!(store.age_order.is_empty());
    }
}
