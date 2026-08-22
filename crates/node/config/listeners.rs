use crate::connectivity::webrtc::DEFAULT_WEBRTC_DIRECT_LISTEN_ADDR;

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;

/// Listener-level transport controls. Dial support remains compiled in so a node can
/// still reach peers over these transports, while unused inbound listeners can be
/// disabled to reduce sockets, protocol maintenance, and wakeups.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ListenerConfig {
    pub tcp: bool,
    pub quic: bool,
    pub websocket: bool,
    pub webrtc_direct: bool,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            tcp: true,
            quic: true,
            websocket: true,
            webrtc_direct: true,
        }
    }
}

impl ListenerConfig {
    #[must_use]
    pub fn allows(&self, addr: &Multiaddr) -> bool {
        let mut is_tcp = false;
        for protocol in addr.iter() {
            match protocol {
                Protocol::Tcp(_) => is_tcp = true,
                Protocol::Quic | Protocol::QuicV1 => return self.quic,
                Protocol::Ws(_) | Protocol::Wss(_) => return self.websocket,
                Protocol::WebRTCDirect | Protocol::P2pWebRtcDirect => return self.webrtc_direct,
                _ => {}
            }
        }
        !is_tcp || self.tcp
    }
}

pub(super) fn default_listen_addresses() -> Vec<String> {
    vec![
        "/ip4/0.0.0.0/udp/4001/quic-v1".to_string(),
        DEFAULT_WEBRTC_DIRECT_LISTEN_ADDR.to_string(),
        "/ip4/0.0.0.0/tcp/4001".to_string(),
        "/ip4/0.0.0.0/tcp/4002/ws".to_string(),
    ]
}
