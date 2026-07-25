use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct AutoConnectRetryState {
    pub(super) attempts: u32,
    pub(super) window_started_unix_secs: u64,
    pub(super) last_attempt_unix_secs: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DhtProviderState {
    pub announce_attempts: usize,
    pub announce_failures: usize,
    pub namespaces_announced: BTreeSet<String>,
    pub provider_queries: usize,
    pub provider_query_failures: usize,
    pub provider_records_found: usize,
    pub provider_queries_finished: usize,
    pub discovered_provider_peers: HashMap<PeerId, BTreeSet<String>>,
    pub auto_connect_attempted_peers: HashSet<PeerId>,
    pub auto_connect_waiting_for_addrs: HashSet<PeerId>,
    pub(super) auto_connect_retry: HashMap<PeerId, AutoConnectRetryState>,
    auto_connect_retry_order: VecDeque<PeerId>,
    discovered_provider_order: VecDeque<PeerId>,
    auto_connect_attempted_order: VecDeque<PeerId>,
    auto_connect_waiting_order: VecDeque<PeerId>,
    pub(super) provider_announce_started_unix_secs: HashMap<String, u64>,
    pub(super) provider_query_started_unix_secs: HashMap<String, u64>,
    provider_addr_lookup_started_unix_secs: HashMap<PeerId, u64>,
    provider_keys_by_namespace: HashMap<String, Vec<(String, kad::RecordKey)>>,
    start_providing_queries: HashMap<QueryId, String>,
    start_providing_query_keys: HashMap<QueryId, String>,
    get_provider_queries: HashMap<QueryId, String>,
    get_provider_query_keys: HashMap<QueryId, String>,
    provider_addr_lookup_queries: HashMap<QueryId, PeerId>,
}

impl DhtProviderState {
    pub(super) fn provider_keys(
        &mut self,
        namespace: &str,
        discovery_cfg: &DiscoveryConfig,
    ) -> Vec<(String, kad::RecordKey)> {
        self.provider_keys_by_namespace
            .entry(namespace.to_string())
            .or_insert_with(|| dht_provider_keys(namespace, discovery_cfg))
            .clone()
    }

    pub(super) fn track_start_providing_for_key(
        &mut self,
        id: QueryId,
        namespace: &str,
        query_key: &str,
    ) {
        self.start_providing_queries
            .insert(id, namespace.to_string());
        self.start_providing_query_keys
            .insert(id, query_key.to_string());
    }

    pub(super) fn track_get_providers_for_key(
        &mut self,
        id: QueryId,
        namespace: &str,
        query_key: &str,
    ) {
        self.get_provider_queries.insert(id, namespace.to_string());
        self.get_provider_query_keys
            .insert(id, query_key.to_string());
    }

    #[must_use]
    pub fn provider_peer_count(&self) -> usize {
        self.discovered_provider_peers.len()
    }

    pub fn record_provider_peer(&mut self, peer: PeerId, namespace: String) -> bool {
        let first_seen = !self.discovered_provider_peers.contains_key(&peer);
        let inserted = self
            .discovered_provider_peers
            .entry(peer)
            .or_default()
            .insert(namespace);
        if first_seen {
            self.discovered_provider_order.push_back(peer);
            self.prune_provider_tracking();
        }
        inserted
    }

    pub fn mark_auto_connect_attempted(&mut self, peer: PeerId) -> bool {
        self.auto_connect_waiting_for_addrs.remove(&peer);
        let inserted = self.auto_connect_attempted_peers.insert(peer);
        if inserted {
            let now = now_unix_secs();
            let first_retry = !self.auto_connect_retry.contains_key(&peer);
            let retry = self.auto_connect_retry.entry(peer).or_default();
            if first_retry {
                self.auto_connect_retry_order.push_back(peer);
            }
            if retry.window_started_unix_secs == 0
                || now.saturating_sub(retry.window_started_unix_secs)
                    >= AUTO_CONNECT_RETRY_WINDOW_SECS
            {
                retry.attempts = 0;
                retry.window_started_unix_secs = now;
            }
            retry.attempts = retry.attempts.saturating_add(1);
            retry.last_attempt_unix_secs = now;
            while self.auto_connect_retry.len() > MAX_TRACKED_PROVIDER_PEERS {
                let Some(evicted) = self.auto_connect_retry_order.pop_front() else {
                    break;
                };
                self.auto_connect_retry.remove(&evicted);
            }
            self.auto_connect_attempted_order.push_back(peer);
            prune_peer_set(
                &mut self.auto_connect_attempted_peers,
                &mut self.auto_connect_attempted_order,
                MAX_TRACKED_PROVIDER_PEERS,
            );
        }
        inserted
    }

    pub fn mark_auto_connect_waiting_for_addrs(&mut self, peer: PeerId) -> bool {
        if self.auto_connect_attempted_peers.contains(&peer) {
            return false;
        }
        let inserted = self.auto_connect_waiting_for_addrs.insert(peer);
        if inserted {
            self.auto_connect_waiting_order.push_back(peer);
            prune_peer_set(
                &mut self.auto_connect_waiting_for_addrs,
                &mut self.auto_connect_waiting_order,
                MAX_TRACKED_PROVIDER_PEERS,
            );
        }
        inserted
    }

    pub fn mark_auto_connect_failed(&mut self, peer: &PeerId) -> bool {
        self.auto_connect_waiting_for_addrs.remove(peer);
        self.auto_connect_attempted_peers.remove(peer)
    }

    /// Make a previously connected discovery peer eligible for the bounded
    /// reconnect policy. The retry history is intentionally retained so a
    /// flapping peer cannot bypass the cooldown or per-window attempt cap.
    pub fn mark_auto_connect_disconnected(&mut self, peer: &PeerId) {
        self.auto_connect_waiting_for_addrs.remove(peer);
        self.auto_connect_attempted_peers.remove(peer);
    }

    #[must_use]
    pub fn should_auto_connect_provider_result(&mut self, peer: &PeerId) -> bool {
        if self.auto_connect_waiting_for_addrs.contains(peer) {
            return false;
        }
        self.auto_connect_retry_allowed(peer)
    }

    #[must_use]
    pub fn should_auto_connect_after_addr_update(&mut self, peer: &PeerId) -> bool {
        self.auto_connect_retry_allowed(peer)
    }

    fn auto_connect_retry_allowed(&mut self, peer: &PeerId) -> bool {
        if self.auto_connect_attempted_peers.contains(peer) {
            return false;
        }
        let now = now_unix_secs();
        let Some(retry) = self.auto_connect_retry.get_mut(peer) else {
            return true;
        };
        if now.saturating_sub(retry.window_started_unix_secs) >= AUTO_CONNECT_RETRY_WINDOW_SECS {
            *retry = AutoConnectRetryState {
                window_started_unix_secs: now,
                ..AutoConnectRetryState::default()
            };
            return true;
        }
        retry.attempts < MAX_AUTO_CONNECT_ATTEMPTS_PER_WINDOW
            && now.saturating_sub(retry.last_attempt_unix_secs) >= AUTO_CONNECT_RETRY_COOLDOWN_SECS
    }

    pub(super) fn complete_start_providing(&mut self, id: &QueryId) -> Option<String> {
        self.start_providing_query_keys.remove(id);
        self.start_providing_queries.remove(id)
    }

    pub fn provider_namespace(&self, id: &QueryId) -> Option<String> {
        self.get_provider_queries.get(id).cloned()
    }

    fn provider_query_key_inflight(&self, query_key: &str) -> bool {
        self.get_provider_query_keys
            .values()
            .any(|active| active == query_key)
    }

    fn provider_announce_key_inflight(&self, query_key: &str) -> bool {
        self.start_providing_query_keys
            .values()
            .any(|active| active == query_key)
    }

    pub(super) fn should_announce_key(
        &self,
        query_key: &str,
        namespace: &str,
        now: u64,
        refresh_interval_secs: u64,
    ) -> bool {
        if self.provider_announce_key_inflight(query_key) {
            return false;
        }
        if !self.namespaces_announced.contains(namespace) {
            return true;
        }
        self.provider_announce_started_unix_secs
            .get(query_key)
            .map(|last| now.saturating_sub(*last) >= refresh_interval_secs)
            .unwrap_or(true)
    }

    pub(super) fn should_query_key(
        &self,
        query_key: &str,
        now: u64,
        refresh_interval_secs: u64,
    ) -> bool {
        if self.provider_query_key_inflight(query_key) {
            return false;
        }
        self.provider_query_started_unix_secs
            .get(query_key)
            .map(|last| now.saturating_sub(*last) >= refresh_interval_secs)
            .unwrap_or(true)
    }

    pub(super) fn complete_get_providers(&mut self, id: &QueryId) -> Option<String> {
        self.get_provider_query_keys.remove(id);
        self.get_provider_queries.remove(id)
    }

    pub fn start_provider_addr_lookup(&mut self, id: QueryId, peer: PeerId, now: u64) {
        self.provider_addr_lookup_queries.insert(id, peer);
        self.provider_addr_lookup_started_unix_secs
            .insert(peer, now);
    }

    #[must_use]
    pub fn provider_addr_lookup_peer(&self, id: &QueryId) -> Option<PeerId> {
        self.provider_addr_lookup_queries.get(id).copied()
    }

    pub fn complete_provider_addr_lookup(&mut self, id: &QueryId) -> Option<PeerId> {
        self.provider_addr_lookup_queries.remove(id)
    }

    #[must_use]
    pub fn should_lookup_provider_addrs(&self, peer: &PeerId, now: u64) -> bool {
        if self
            .provider_addr_lookup_queries
            .values()
            .any(|inflight| inflight == peer)
        {
            return false;
        }
        self.provider_addr_lookup_started_unix_secs
            .get(peer)
            .map(|last| now.saturating_sub(*last) >= PROVIDER_ADDR_LOOKUP_COOLDOWN_SECS)
            .unwrap_or(true)
    }

    pub fn start_provider_addr_lookup_if_due(
        &mut self,
        swarm: &mut Swarm<MeshBehaviour>,
        peer: PeerId,
    ) -> bool {
        let now = now_unix_secs();
        if !self.should_lookup_provider_addrs(&peer, now) {
            return false;
        }
        let query_id = swarm
            .behaviour_mut()
            .kademlia
            .get_closest_peers(peer.to_bytes());
        self.start_provider_addr_lookup(query_id, peer, now);
        true
    }

    fn prune_provider_tracking(&mut self) {
        while self.discovered_provider_peers.len() > MAX_TRACKED_PROVIDER_PEERS {
            let Some(evicted) = self.discovered_provider_order.pop_front() else {
                break;
            };
            if self.discovered_provider_peers.remove(&evicted).is_some() {
                self.auto_connect_attempted_peers.remove(&evicted);
                self.auto_connect_waiting_for_addrs.remove(&evicted);
                self.provider_addr_lookup_started_unix_secs.remove(&evicted);
                self.provider_addr_lookup_queries
                    .retain(|_, peer| peer != &evicted);
            }
        }
    }
}

fn prune_peer_set(set: &mut HashSet<PeerId>, order: &mut VecDeque<PeerId>, max_entries: usize) {
    while set.len() > max_entries {
        let Some(evicted) = order.pop_front() else {
            break;
        };
        set.remove(&evicted);
    }
}
