#![no_main]

use baals::Transaction;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(tx) = bincode::deserialize::<Transaction>(data) {
        let _ = tx.calculate_hash();
        let _ = tx.verify_signature();
    }
});
