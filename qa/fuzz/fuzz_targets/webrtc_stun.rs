#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = libp2p_webrtc::tokio::fuzz_stun_ufrag(data, true);
    let _ = libp2p_webrtc::tokio::fuzz_stun_ufrag(data, false);
});
