#![no_main]

use libfuzzer_sys::fuzz_target;
use p2p_net::NodeConfig;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 * 1024 {
        return;
    }
    if let Ok(cfg) = serde_json::from_slice::<NodeConfig>(data) {
        let _ = cfg.validate();
        let _ = cfg.to_pretty_json();
    }
});
