#![no_main]

use electrum_private_relay::protocol::classify;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|frame: &[u8]| {
    let _ = classify(frame);
});
