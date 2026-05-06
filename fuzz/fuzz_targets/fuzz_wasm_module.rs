#![no_main]

use baals::{BaaLSContractEngine, SledStorage};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = BaaLSContractEngine::<SledStorage>::scan_for_float_opcodes(data);
});
