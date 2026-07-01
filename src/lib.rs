//! Standalone all-in-one libp2p node glue.
//!
//! - **`stack`**: transport, `MeshBehaviour`, and discovery helpers.
//! - **`connectivity`**: NAT/relay state and on-disk peer address cache.
//! - **`protocol`**: heartbeat gossip and lightweight reputation.
//! - **`common`**: `NetError` and shared helpers.
//! - **`node`**: Tokio orchestration (`start_node`) and snapshots.
//! - **`platform`**: storage/runtime adapters for desktop and mobile shells.

#![forbid(unsafe_code)]

pub mod common;
pub mod connectivity;
pub mod platform;
pub mod protocol;
pub mod stack;

mod node;

pub use common::error::NetError;
pub use connectivity::discovery::DiscoveryConfig;
pub use connectivity::dcutr::DcutrPolicy;
pub use connectivity::dns::{
    DnsaddrConfig, DEFAULT_DNSADDR_DOH_ENDPOINT, DEFAULT_DNSADDR_TIMEOUT_SECS,
};
pub use connectivity::limits::{ConnectionCapState, ConnectionLimitsConfig};
pub use connectivity::mediator::MediatorConfig;
pub use connectivity::relay::{
    is_p2p_circuit_addr, relay_peer_id, relay_reservation_addr, RelayAccess, RelaySchedule,
    RelayServiceConfig, RelayServiceHealth, RelayWindow,
};
pub use connectivity::relay_discovery::{
    select_startup_relays, RelayCandidate, RelayCandidateSource, RelayDiscoveryPolicy,
    RelaySelectionPlan,
};
pub use connectivity::rendezvous::{RendezvousConfig, RendezvousState};
pub use libp2p::{Multiaddr, PeerId};
pub use platform::{
    DesktopPlatformRuntime, MemoryNodeStorage, MobilePlatformRuntime, NodeStorage, PlatformRuntime,
};
pub use node::{
    apply_resolved_capabilities, resolve_node_config, snapshot_to_json,
    snapshot_to_prometheus_metrics, start_node, start_node_with_platform, BehaviourSet,
    EnvironmentConfig, EnvironmentReport, NatKind, NetworkReachability, NodeConfig, NodeHandle,
    NodeProfile, NodeRole, NodeSnapshot, PlatformKind, ResolvedNodeConfig,
};
pub use protocol::pulse::{
    heartbeat_topic, validate_heartbeat_wire, verify_heartbeat, verify_heartbeat_with_config,
    HeartbeatEnvelope, HeartbeatReplayCache, HeartbeatValidationDecision,
    HeartbeatValidationResult, MessageSecurityConfig, ReputationConfig,
};
