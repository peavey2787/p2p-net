//! Fast-tick TURN upkeep: credential refreshes and broker re-requests.

use std::sync::Arc;

use libp2p::Swarm;
use tokio::sync::Mutex;

use crate::stack::MeshBehaviour;

use super::super::events;
use super::super::snapshot::NodeSnapshot;
use super::RuntimeState;

/// Collect finished TURN credential fetches, start due ones and re-ask
/// brokers whose credentials are about to expire. Cheap when nothing is due,
/// so it runs on the fast observability tick. Returns true when the TURN
/// snapshot changed.
pub(super) async fn maintain_turn(
    swarm: &mut Swarm<MeshBehaviour>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    runtime_state: &mut RuntimeState,
) -> bool {
    let now = web_time::Instant::now();
    let (due, pulses) = runtime_state.turn.maintain(now);
    for peer in due {
        events::request_turn_credentials(peer, swarm, &mut runtime_state.turn, now);
    }
    for pulse in pulses {
        runtime_state.observability.pulse(pulse);
    }
    let mut guard = snapshot.lock().await;
    if guard.turn == *runtime_state.turn.snapshot() {
        return false;
    }
    guard.turn = runtime_state.turn.snapshot().clone();
    true
}
