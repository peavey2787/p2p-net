use std::sync::Arc;

use tokio::sync::Mutex;

use crate::connectivity::relay::RelayState;
use super::super::types::NodeSnapshot;

use super::super::push_pulse;

pub(crate) async fn handle_event(
    ev: libp2p::dcutr::Event,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
) {
    relay_state.dcutr_attempts = relay_state.dcutr_attempts.saturating_add(1);
    let debug = format!("{ev:?}");
    let lower = debug.to_ascii_lowercase();
    if lower.contains("success") || lower.contains("established") {
        relay_state.dcutr_successes = relay_state.dcutr_successes.saturating_add(1);
    }

    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(&mut guard.pulses, format!("dcutr event {debug}"));
}
