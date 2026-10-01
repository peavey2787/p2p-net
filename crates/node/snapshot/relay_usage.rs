use super::NodeSnapshot;
use crate::connectivity::relay::RelayState;

impl NodeSnapshot {
    /// Relay fields that live outside the size-budgeted snapshot core.
    pub(super) fn apply_relay_extensions(&mut self, relay_state: &RelayState) {
        self.relay_bytes_forwarded = relay_state.relay_bytes_forwarded;
        self.relay_usage = relay_state.relay_usage_snapshot();
        self.apply_private_relayed(relay_state);
    }
}
