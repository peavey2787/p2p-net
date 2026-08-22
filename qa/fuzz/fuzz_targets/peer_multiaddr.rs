#![no_main]

use libfuzzer_sys::fuzz_target;
use p2p_net::connectivity::{dns, webrtc};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return; };
    if let Ok(addr) = text.parse::<libp2p::Multiaddr>() {
        let _ = dns::has_any_dns(&addr);
        let _ = dns::has_dnsaddr(&addr);
        let _ = dns::has_resolvable_dns(&addr);
        let _ = webrtc::is_webrtc_direct_addr(&addr);
        let _ = webrtc::has_webrtc_direct_certhash(&addr);
    }
});
