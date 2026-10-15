//! TURN for browser WebRTC: relay sources and credential providers
//! (Cloudflare, generic HTTP, shared secret), the built-in TURN server, and
//! the libp2p credential broker that lets browsers use TURN without holding
//! any secret. ICE policy and STUN belong to `browser_webrtc`.

mod broker;
mod client;
mod config;
mod credentials;
mod provider;
mod refresh;
mod secret;
#[cfg(not(target_arch = "wasm32"))]
mod server;
mod status;

pub use broker::{
    CredentialRequest, CredentialResponse, TurnBrokerState, TurnBrokerStatus, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES, TURN_CREDENTIALS_PROTOCOL,
};
pub use client::{BrokerOffer, TurnClientState, TurnClientStatus};
pub use config::{
    DiscoveryTrust, TurnBrokerConfig, TurnClientConfig, TurnConfig, TurnServerConfig, TurnSource,
    MAX_ICE_SERVERS,
};
pub use credentials::{
    TurnCredentials, DEFAULT_CREDENTIAL_TTL_SECS, MAX_CREDENTIAL_TTL_SECS,
    MAX_ICE_SERVERS_PER_SOURCE, MIN_CREDENTIAL_TTL_SECS,
};
pub use provider::{
    CloudflareProviderConfig, HttpProviderConfig, SharedSecretProviderConfig, TurnProviderConfig,
    CLOUDFLARE_TURN_HOST,
};
pub(crate) use refresh::ProviderRefresher;
pub use secret::SecretRef;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use server::RunningTurnServer;
pub use status::{TurnServerState, TurnSnapshot};
