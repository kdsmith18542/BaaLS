use serde::{Deserialize, Serialize};

/// Captures the pre-image (old value) of a storage key before a write.
/// On rollback replay, restores the key to its old value, or deletes it if
/// old_value is None (meaning the key did not exist before the write).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum RollbackOp {
    Account(Vec<u8>, Option<Vec<u8>>),
    ChainState(Vec<u8>, Option<Vec<u8>>),
    ContractStorage(Vec<u8>, Option<Vec<u8>>),
    ContractCode(Vec<u8>, Option<Vec<u8>>),
    ContractDeployer(Vec<u8>, Option<Vec<u8>>),
    TxStatus(Vec<u8>, Option<Vec<u8>>),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RollbackLog {
    pub block_hash: [u8; 32],
    pub block_index: u64,
    pub ops: Vec<RollbackOp>,
}
