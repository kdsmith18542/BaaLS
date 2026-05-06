#![no_main]

use baals::Block;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(block) = bincode::deserialize::<Block>(data) {
        let _ = block.calculate_hash();
    }
});
