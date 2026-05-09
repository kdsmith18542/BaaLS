#![no_main]

use baals::{Block, Transaction};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Try deserializing as a block and exercising validation
    if let Ok(block) = bincode::deserialize::<Block>(data) {
        let _ = block.calculate_hash();
        let tx_count = block.transactions.len();
        for tx in &block.transactions {
            let _ = tx.calculate_hash();
            let _ = tx.verify_signature();
        }
        let _ = tx_count;
    }
    // Also try as a standalone transaction
    if let Ok(tx) = bincode::deserialize::<Transaction>(data) {
        let _ = tx.calculate_hash();
        let _ = tx.verify_signature();
        let _ = tx.payload_size_estimate();
    }
});
