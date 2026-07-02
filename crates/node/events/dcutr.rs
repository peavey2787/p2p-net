use std::sync::Arc;

use tokio::sync::Mutex;

use crate::connectivity::dcutr::DcutrPolicy;
use crate::connectivity::relay::RelayState;

use super::super::push_pulse;
use super::super::snapshot::NodeSnapshot;

pub(crate) async fn handle_event(
    ev: libp2p::dcutr::Event,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
    policy: &DcutrPolicy,
) {
    relay_state.dcutr_enabled = policy.enabled;
    if !policy.enabled {
        let mut guard = snapshot.lock().await;
        guard.apply_relay_state(relay_state);
        push_pulse(&mut guard.pulses, format!("dcutr event ignored while disabled {ev:?}"));
        return;
    }

    relay_state.dcutr_attempts = relay_state.dcutr_attempts.saturating_add(1);
    let debug = format!("{ev:?}");
    let lower = debug.to_ascii_lowercase();
    if lower.contains("success") || lower.contains("established") {
        relay_state.dcutr_successes = relay_state.dcutr_successes.saturating_add(1);
    } else if lower.contains("fail")
        || lower.contains("error")
        || lower.contains("denied")
        || lower.contains("unsupported")
    {
        relay_state.dcutr_failures = relay_state.dcutr_failures.saturating_add(1);
        if policy.keep_relay_fallback {
            relay_state.dcutr_relay_fallbacks = relay_state.dcutr_relay_fallbacks.saturating_add(1);
        }
    }

    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(
        &mut guard.pulses,
        format!(
            "dcutr event {debug}; keep_relay_fallback={} retry_interval_secs={} max_attempts_per_peer={}",
            policy.keep_relay_fallback, policy.retry_interval_secs, policy.max_attempts_per_peer
        ),
    );
}
