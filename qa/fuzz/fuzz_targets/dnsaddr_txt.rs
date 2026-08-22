#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = p2p_net::connectivity::dns::decode_dnsaddr_txt_value(text);
    }
});
