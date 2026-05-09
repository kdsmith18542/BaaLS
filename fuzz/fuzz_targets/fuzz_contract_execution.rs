#![no_main]

use baals::BaaLSContractEngine;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Test WASM module scanning for malicious opcodes
    let _ = BaaLSContractEngine::<baals::SledStorage>::scan_for_float_opcodes(data);
});
