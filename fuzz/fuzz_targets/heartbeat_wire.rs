#![no_main]

use libfuzzer_sys::fuzz_target;
use p2p_net::{validate_heartbeat_wire, HeartbeatReplayCache, MessageSecurityConfig};

fuzz_target!(|data: &[u8]| {
    let peer = p2p_net::PeerId::random();
    let cfg = MessageSecurityConfig {
        max_heartbeat_wire_bytes: 4096,
        ..MessageSecurityConfig::default()
    };
    let mut cache = HeartbeatReplayCache::new(&cfg);
    let _ = validate_heartbeat_wire(peer, data, 0, &cfg, &mut cache);
});
