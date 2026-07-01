#![no_main]

use std::fs;

use libfuzzer_sys::fuzz_target;
use p2p_net::connectivity::peer_cache;
use p2p_net::DiscoveryConfig;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 * 1024 {
        return;
    }
    let path = std::env::temp_dir().join(format!(
        "p2p-net-fuzz-cache-{}-{}.json",
        std::process::id(),
        data.len()
    ));
    let _ = fs::write(&path, data);
    let cfg = DiscoveryConfig {
        peer_cache_path: path.to_string_lossy().to_string(),
        ..DiscoveryConfig::default()
    };
    let _ = peer_cache::load_entries(&cfg);
    let _ = peer_cache::load_last_addrs(&cfg, 32);
    let _ = fs::remove_file(path);
});
