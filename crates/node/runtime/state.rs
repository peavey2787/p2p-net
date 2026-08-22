use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::DhtProviderState;
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::peer_cache::PeerCacheWriteBatch;
use crate::connectivity::relay::RelayReservationPlan;
use crate::connectivity::relay_discovery::RelaySelectionPlan;
use crate::connectivity::rendezvous::RendezvousState;
use crate::protocol::app_security::AppMessageReplayCache;
use crate::protocol::pulse::HeartbeatReplayCache;
use crate::protocol::reputation::ReputationStore;
use crate::stack::IdentifyAddressState;

use super::super::config::NodeConfig;
use super::super::dial::AutoDialStats;
use super::super::events::ObservabilityBatch;
use super::super::profile::ResolvedNodeConfig;
use super::super::runtime_maintenance;
use super::{dht_schedule, RuntimeState};

impl RuntimeState {
    pub(super) fn new(
        cfg: &NodeConfig,
        resolved_config: &ResolvedNodeConfig,
        relay_reservation_plan: RelayReservationPlan,
        relay_selection_plan: RelaySelectionPlan,
        rendezvous_state: RendezvousState,
        dht_state: DhtProviderState,
        peer_book: PeerBook,
    ) -> Self {
        Self {
            rep: ReputationStore::new(cfg.message_security.reputation.clone()),
            replay_cache: HeartbeatReplayCache::new(&cfg.message_security),
            app_replay_cache: AppMessageReplayCache::new(&cfg.message_security),
            relay_state: runtime_maintenance::initial_relay_state(
                cfg,
                resolved_config,
                &relay_reservation_plan,
                &relay_selection_plan,
            ),
            rendezvous_state,
            dht_state,
            peer_book,
            pending_connections: PendingConnectionPlans::default(),
            auto_dial_stats: AutoDialStats::default(),
            connection_caps: ConnectionCapState::new(&cfg.connection_limits),
            app_topic_hashes: Vec::new(),
            metrics: crate::api::NodeMetrics::default(),
            identify_addresses: IdentifyAddressState::default(),
            observability: ObservabilityBatch::default(),
            peer_cache_writes: PeerCacheWriteBatch::default(),
            dht_refresh_schedule: dht_schedule::DhtRefreshSchedule::new(
                cfg.discovery.dht.refresh_interval_secs,
            ),
            last_peer_cache_flush: std::time::Instant::now(),
        }
    }

    pub(super) fn flush_peer_cache(
        &mut self,
        cfg: &NodeConfig,
        storage: &dyn crate::platform::NodeStorage,
    ) {
        if self.peer_cache_writes.is_empty() {
            return;
        }
        self.peer_cache_writes.flush(&cfg.discovery, storage);
        self.last_peer_cache_flush = std::time::Instant::now();
    }
}
