//! Shared libp2p node core with profile-driven capabilities.
//!
//! - **`api`**: stable app primitives, telemetry metrics, and message envelopes.
//! - **`stack`**: transport, `MeshBehaviour`, and discovery helpers.
//! - **`connectivity`**: NAT/relay state and on-disk peer address cache.
//! - **`protocol`**: heartbeat gossip and lightweight reputation.
//! - **`common`**: `NetError` and shared helpers.
//! - **`node`**: Tokio orchestration (`start_node`) and snapshots.
//! - **`platform`**: storage/runtime adapters for desktop and mobile shells.
//! - **`bindings`**: JSON/enum-oriented facade for app-shell bindings.

#![forbid(unsafe_code)]

pub mod api;
pub mod bindings;
pub mod common;
pub mod connectivity;
pub mod platform;
pub mod protocol;
pub mod stack;

mod node;

pub use api::{
    app_ident_topic, app_topic_name, decode_app_message, encode_app_message, normalize_app_topic,
    validate_app_message, AppMessage, AppSubscription, BandwidthMetrics, ComputeMetrics,
    NodeMetrics, P2PNode, PeerBandwidth, PeerInfo, PeerSource, StorageMetrics, TopicBandwidth,
    APP_MESSAGE_SCHEMA_VERSION, APP_TOPIC_PREFIX, MAX_APP_MESSAGE_BYTES, MAX_APP_TOPIC_LEN,
};
pub use bindings::{
    binding_support_matrix, node_config_from_json, node_config_to_json,
    node_snapshot_to_json_string, prepare_binding_start_plan, BindingPlatformRuntime,
    BindingRuntimeSpec, BindingStartPlan, BindingStorageRequirement, BindingStorageStrategy,
    BindingSupportMatrix, BindingTarget, BindingTargetInfo,
};
pub use common::error::NetError;
pub use connectivity::connection_strategy::{
    build_connection_plan, ConnectionAttempt, ConnectionAttemptKind, ConnectionPlan,
    PendingConnectionPlans,
};
pub use connectivity::dcutr::DcutrPolicy;
pub use connectivity::dht::{
    dht_record_key, start_dht_namespace_discovery, DhtDiscoveryConfig, DhtNamespacePlan,
    DhtProviderState,
};
pub use connectivity::discovery::DiscoveryConfig;
pub use connectivity::dns::{
    DnsaddrConfig, DEFAULT_DNSADDR_DOH_ENDPOINT, DEFAULT_DNSADDR_TIMEOUT_SECS,
};
pub use connectivity::limits::{ConnectionCapState, ConnectionLimitsConfig};
pub use connectivity::mediator::MediatorConfig;
pub use connectivity::namespace::{
    build_discovery_namespace, discovery_tag_hash_hex, DiscoveryNamespace,
    DiscoveryNamespaceConfig, DiscoveryNamespacePrivacy, DISCOVERY_NAMESPACE_PREFIX,
};
pub use connectivity::peer_book::{PeerBook, PeerRecord, DEFAULT_MAX_PEER_BOOK_RECORDS};
pub use connectivity::public_fallback::{
    PublicBootstrapConfig, PublicFallbackDecision, PublicFallbackMode,
    DEFAULT_PUBLIC_BOOTSTRAP_SEED_PEERS, DEFAULT_PUBLIC_RELAY_PEERS,
    DEFAULT_PUBLIC_RENDEZVOUS_PEERS,
};
pub use connectivity::relay::{
    is_p2p_circuit_addr, relay_peer_id, relay_reservation_addr, RelayAccess, RelaySchedule,
    RelayServiceConfig, RelayServiceHealth, RelayWindow,
};
pub use connectivity::relay_discovery::{
    select_startup_relays, RelayCandidate, RelayCandidateSource, RelayDiscoveryPolicy,
    RelaySelectionPlan,
};
pub use connectivity::rendezvous::{RendezvousConfig, RendezvousState};
pub use connectivity::webrtc::{
    has_webrtc_direct_certhash, is_webrtc_direct_addr, DEFAULT_WEBRTC_DIRECT_LISTEN_ADDR,
    WEBRTC_DIRECT_TRANSPORT,
};
pub use libp2p::{Multiaddr, PeerId};
pub use node::{
    apply_resolved_capabilities, resolve_node_config, snapshot_to_json,
    snapshot_to_prometheus_metrics, start_node, start_node_with_platform, BehaviourSet,
    EnvironmentConfig, EnvironmentReport, ListenerConfig, NatKind, NetworkReachability, NodeConfig,
    NodeHandle,
    NodeProfile, NodeRole, NodeSnapshot, PlatformKind, PublicIpProbeConfig, ResolvedNodeConfig,
};
pub use platform::{
    DesktopPlatformRuntime, MemoryNodeStorage, MobilePlatformRuntime, NodeStorage, PlatformRuntime,
};
pub use protocol::pulse::{
    encode_heartbeat_wire, heartbeat_topic, validate_heartbeat_wire, verify_heartbeat,
    verify_heartbeat_with_config,
    HeartbeatEnvelope, HeartbeatReplayCache, HeartbeatValidationDecision,
    HeartbeatValidationResult, MessageSecurityConfig, ReputationConfig,
};
