//! Native WebRTC-direct multiaddr helpers.
//!
//! Browser-compatible WebRTC is a first-class libp2p swarm transport in this
//! crate through `libp2p-webrtc`. These helpers keep address classification DRY
//! across relay discovery, probes, and tests without introducing a parallel
//! peer-routing or app-message path.

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;

pub const WEBRTC_DIRECT_TRANSPORT: &str = "webrtc-direct";
pub const DEFAULT_WEBRTC_DIRECT_LISTEN_ADDR: &str = "/ip4/0.0.0.0/udp/4003/webrtc-direct";

/// Return true when a multiaddr uses the native libp2p WebRTC-direct transport.
#[must_use]
pub fn is_webrtc_direct_addr(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::WebRTCDirect | Protocol::P2pWebRtcDirect))
}

/// Return true when the address contains WebRTC-direct plus its certificate hash.
///
/// Dialable WebRTC-direct addresses include `/certhash/...` after
/// `/webrtc-direct`; listen requests usually omit it until libp2p emits the
/// concrete listen address.
#[must_use]
pub fn has_webrtc_direct_certhash(addr: &Multiaddr) -> bool {
    let mut saw_webrtc_direct = false;
    for protocol in addr.iter() {
        match protocol {
            Protocol::WebRTCDirect | Protocol::P2pWebRtcDirect => saw_webrtc_direct = true,
            Protocol::Certhash(_) if saw_webrtc_direct => return true,
            _ => {}
        }
    }
    false
}
