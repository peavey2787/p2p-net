//! Browser WebRTC settings for the relay-assisted direct upgrade.

use serde::{Deserialize, Serialize};

/// ICE configuration for browser-to-browser `/webrtc` connections.
///
/// Nothing here contacts a third-party service by default: with no ICE
/// servers, browsers use their own host candidates only, and a peer pair that
/// cannot connect that way keeps using its Circuit Relay connection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserWebRtcConfig {
    /// `stun:`/`turn:` URLs, e.g. STUN served by the operator's own relays.
    #[serde(default)]
    pub ice_servers: Vec<String>,
    /// ICE transport policy: `all` (default) or `relay` (TURN only; hides
    /// local addresses, and without TURN servers disables direct upgrades).
    #[serde(default)]
    pub ice_transport_policy: IceTransportPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IceTransportPolicy {
    #[default]
    All,
    Relay,
}
