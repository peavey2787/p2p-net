#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = p2p_net::api::fuzz_decode_internal_fragment(data);
});
