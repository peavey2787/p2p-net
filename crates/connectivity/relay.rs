//! Circuit relay configuration, state, scheduling, and address helpers.

mod address;
mod config;
mod policy;
mod schedule;
mod state;

pub use address::{is_p2p_circuit_addr, relay_peer_id, relay_reservation_addr};
pub use config::{RelayAccess, RelayServiceConfig};
pub use policy::classify_relay_denial;
pub use schedule::{RelaySchedule, RelayWindow};
pub use state::{update_nat_state, RelayReservationPlan, RelayServiceHealth, RelayState};
