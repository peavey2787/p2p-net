//! Batched snapshot and peer-cache observability flushing.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::platform::NodeStorage;

use super::super::config::NodeConfig;
use super::super::events;
use super::super::snapshot::NodeSnapshot;
use super::RuntimeState;
use super::PEER_CACHE_FLUSH_INTERVAL;

pub(super) async fn flush_observability(
    cfg: &NodeConfig,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    storage: &dyn NodeStorage,
    runtime_state: &mut RuntimeState,
) -> bool {
    if runtime_state.last_peer_cache_flush.elapsed() >= PEER_CACHE_FLUSH_INTERVAL {
        runtime_state.flush_peer_cache(cfg, storage);
    }
    if runtime_state.observability.is_empty() {
        return false;
    }
    let mut guard = snapshot.lock().await;
    events::flush_observability_snapshot(
        &mut guard,
        &mut runtime_state.observability,
        &runtime_state.dht_state,
        &runtime_state.peer_book,
        &runtime_state.auto_dial_stats,
        &runtime_state.pending_connections,
        cfg.discovery.public_bootstrap.auto_connect_discovered_peers,
    );
    true
}
