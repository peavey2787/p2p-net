//! Long-running node runtime facade.
//!
//! Startup code constructs the swarm and initial discovery state, then hands
//! ownership to the focused runtime driver.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use libp2p::gossipsub::{IdentTopic, TopicHash};
use libp2p::identity::Keypair;
use libp2p::{Multiaddr, PeerId, Swarm};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::api::{AppMessage, NodeMetrics};
use crate::connectivity::connection_strategy::PendingConnectionPlans;
use crate::connectivity::dht::DhtProviderState;
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::peer_cache::PeerCacheWriteBatch;
use crate::connectivity::relay::{RelayReservationPlan, RelayState};
use crate::connectivity::relay_discovery::RelaySelectionPlan;
use crate::connectivity::rendezvous::RendezvousState;
use crate::platform::NodeStorage;
use crate::protocol::app_security::AppMessageReplayCache;
use crate::protocol::pulse::HeartbeatReplayCache;
use crate::protocol::reputation::ReputationStore;
use crate::stack::{IdentifyAddressState, MeshBehaviour};

use super::config::NodeConfig;
use super::dial::AutoDialStats;
use super::events::ObservabilityBatch;
use super::handle::NodeCommand;
use super::profile::ResolvedNodeConfig;
use super::runtime_maintenance;
use super::snapshot::NodeSnapshot;

mod dht_schedule;
mod driver;
mod observability;
mod periodic;

pub(super) const PEER_CACHE_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) struct NodeRuntimeContext {
    pub(crate) cfg: NodeConfig,
    pub(crate) resolved_config: ResolvedNodeConfig,
    pub(crate) swarm: Swarm<MeshBehaviour>,
    pub(crate) local_peer: PeerId,
    pub(crate) discovery_signing_key: Keypair,
    pub(crate) heartbeat_topic: IdentTopic,
    pub(crate) snapshot: Arc<Mutex<NodeSnapshot>>,
    pub(crate) snapshot_revision: Arc<AtomicU64>,
    pub(crate) storage: Arc<dyn NodeStorage>,
    pub(crate) rendezvous_peers: Vec<Multiaddr>,
    pub(crate) relay_reservation_plan: RelayReservationPlan,
    pub(crate) relay_selection_plan: RelaySelectionPlan,
    pub(crate) rendezvous_state: RendezvousState,
    pub(crate) dht_state: DhtProviderState,
    pub(crate) peer_book: PeerBook,
    pub(crate) shutdown_rx: mpsc::Receiver<()>,
    pub(crate) command_rx: mpsc::Receiver<NodeCommand>,
    pub(crate) messages_tx: broadcast::Sender<AppMessage>,
}

pub(super) struct RuntimeState {
    pub(super) rep: ReputationStore,
    pub(super) replay_cache: HeartbeatReplayCache,
    pub(super) app_replay_cache: AppMessageReplayCache,
    pub(super) relay_state: RelayState,
    pub(super) rendezvous_state: RendezvousState,
    pub(super) dht_state: DhtProviderState,
    pub(super) peer_book: PeerBook,
    pub(super) pending_connections: PendingConnectionPlans,
    pub(super) auto_dial_stats: AutoDialStats,
    pub(super) connection_caps: ConnectionCapState,
    pub(super) app_topic_hashes: Vec<TopicHash>,
    pub(super) metrics: NodeMetrics,
    pub(super) identify_addresses: IdentifyAddressState,
    pub(super) observability: ObservabilityBatch,
    pub(super) peer_cache_writes: PeerCacheWriteBatch,
    dht_refresh_schedule: dht_schedule::DhtRefreshSchedule,
    pub(super) last_peer_cache_flush: std::time::Instant,
}

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
            metrics: NodeMetrics::default(),
            identify_addresses: IdentifyAddressState::default(),
            observability: ObservabilityBatch::default(),
            peer_cache_writes: PeerCacheWriteBatch::default(),
            dht_refresh_schedule: dht_schedule::DhtRefreshSchedule::new(
                cfg.discovery.dht.refresh_interval_secs,
            ),
            last_peer_cache_flush: std::time::Instant::now(),
        }
    }

    pub(super) fn flush_peer_cache(&mut self, cfg: &NodeConfig, storage: &dyn NodeStorage) {
        if self.peer_cache_writes.is_empty() {
            return;
        }
        self.peer_cache_writes.flush(&cfg.discovery, storage);
        self.last_peer_cache_flush = std::time::Instant::now();
    }
}

pub(crate) fn spawn_node_runtime(ctx: NodeRuntimeContext) -> JoinHandle<()> {
    tokio::spawn(driver::run_node_runtime(ctx))
}
