use std::collections::HashMap;
use std::time::{Duration, Instant};

use libp2p::PeerId;

use crate::protocol::pulse::ReputationConfig;

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

#[derive(Debug)]
pub struct ReputationStore {
    by_peer: HashMap<PeerId, ReputationEntry>,
    cfg: ReputationConfig,
}

impl Default for ReputationStore {
    fn default() -> Self {
        Self::new(ReputationConfig::default())
    }
}

impl ReputationStore {
    #[must_use]
    pub fn new(cfg: ReputationConfig) -> Self {
        Self {
            by_peer: HashMap::new(),
            cfg,
        }
    }

    pub fn accept(&mut self, peer: PeerId) {
        let ent = self.by_peer.entry(peer).or_default();
        ent.accepted = ent.accepted.saturating_add(1);
        ent.score = ent.score.saturating_add(self.cfg.accept_reward);
        ent.last_updated = Instant::now();
    }

    pub fn penalize_invalid(&mut self, peer: PeerId) {
        let ent = self.by_peer.entry(peer).or_default();
        ent.invalid = ent.invalid.saturating_add(1);
        ent.score = ent.score.saturating_sub(self.cfg.invalid_penalty);
        ent.last_updated = Instant::now();
    }

    pub fn ignore_duplicate(&mut self, peer: PeerId) {
        let ent = self.by_peer.entry(peer).or_default();
        ent.duplicate_ignored = ent.duplicate_ignored.saturating_add(1);
        ent.last_updated = Instant::now();
    }

    pub fn tick_decay(&mut self) {
        let now = Instant::now();
        let interval = Duration::from_secs(self.cfg.decay_interval_secs.max(1));
        let step = self.cfg.decay_step.max(1);
        for ent in self.by_peer.values_mut() {
            if now.duration_since(ent.last_updated) >= interval {
                if ent.score < 0 {
                    ent.score = ent.score.saturating_add(step).min(0);
                } else if ent.score > 0 {
                    ent.score = ent.score.saturating_sub(step).max(0);
                }
                ent.last_updated = now;
            }
        }
    }
}
