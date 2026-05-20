use crate::redb_storage::RedbStorage;
use crate::storage::{
    BackupManifest, ContractEvents, SledStorage, Storage, StorageBatch, StorageError, StorageStats,
};
use crate::types::{Account, Block, ChainState, ContractId, PublicKey, Transaction};

/// Wraps either storage backend, dispatching all Storage trait methods.
/// Enables runtime choice between SledStorage and RedbStorage.
#[derive(Clone)]
pub enum AnyStorage {
    Sled(SledStorage),
    Redb(RedbStorage),
}

macro_rules! dispatch {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            AnyStorage::Sled(s) => s.$method($($arg),*),
            AnyStorage::Redb(s) => s.$method($($arg),*),
        }
    };
}

impl Storage for AnyStorage {
    fn clone_storage(&self) -> Box<dyn Storage> {
        Box::new(self.clone())
    }
    fn put_block(&self, block: &Block) -> Result<(), StorageError> {
        dispatch!(self, put_block, block)
    }
    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StorageError> {
        dispatch!(self, get_block, hash)
    }
    fn get_block_checksum(&self, hash: &[u8; 32]) -> Result<Option<[u8; 32]>, StorageError> {
        dispatch!(self, get_block_checksum, hash)
    }
    fn get_latest_block(&self) -> Result<Option<Block>, StorageError> {
        dispatch!(self, get_latest_block)
    }
    fn get_chain_height(&self) -> Result<u64, StorageError> {
        dispatch!(self, get_chain_height)
    }
    fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError> {
        dispatch!(self, get_block_by_height, height)
    }
    fn put_transaction(&self, tx: &Transaction) -> Result<(), StorageError> {
        dispatch!(self, put_transaction, tx)
    }
    fn get_transaction(&self, tx_hash: &[u8; 32]) -> Result<Option<Transaction>, StorageError> {
        dispatch!(self, get_transaction, tx_hash)
    }
    fn get_pending_transactions(&self) -> Result<Vec<Transaction>, StorageError> {
        dispatch!(self, get_pending_transactions)
    }
    fn put_pending_transaction(&self, tx: &Transaction) -> Result<(), StorageError> {
        dispatch!(self, put_pending_transaction, tx)
    }
    fn remove_pending_transaction(&self, tx_hash: &[u8; 32]) -> Result<(), StorageError> {
        dispatch!(self, remove_pending_transaction, tx_hash)
    }
    fn index_transaction(
        &self,
        tx_hash: &[u8; 32],
        block_hash: &[u8; 32],
        tx_index_in_block: u32,
    ) -> Result<(), StorageError> {
        dispatch!(self, index_transaction, tx_hash, block_hash, tx_index_in_block)
    }
    fn get_transaction_by_id(
        &self,
        tx_hash: &[u8; 32],
    ) -> Result<Option<(Block, Transaction)>, StorageError> {
        dispatch!(self, get_transaction_by_id, tx_hash)
    }
    fn get_transaction_status(
        &self,
        tx_hash: &[u8; 32],
    ) -> Result<Option<crate::types::TransactionStatus>, StorageError> {
        dispatch!(self, get_transaction_status, tx_hash)
    }
    fn put_receipt(&self, receipt: &crate::types::TransactionReceipt) -> Result<(), StorageError> {
        dispatch!(self, put_receipt, receipt)
    }
    fn get_receipt(&self, tx_hash: &[u8; 32]) -> Result<Option<crate::types::TransactionReceipt>, StorageError> {
        dispatch!(self, get_receipt, tx_hash)
    }
    fn get_transactions_by_block(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<Vec<Transaction>, StorageError> {
        dispatch!(self, get_transactions_by_block, block_hash)
    }
    fn get_transactions_by_address(
        &self,
        address: &PublicKey,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError> {
        dispatch!(self, get_transactions_by_address, address, limit)
    }
    fn get_transactions_by_height_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<Transaction>, StorageError> {
        dispatch!(self, get_transactions_by_height_range, from, to)
    }
    fn get_block_hashes_by_height_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<[u8; 32]>, StorageError> {
        dispatch!(self, get_block_hashes_by_height_range, from, to)
    }
    fn get_account_transaction_count(&self, address: &PublicKey) -> Result<u64, StorageError> {
        dispatch!(self, get_account_transaction_count, address)
    }
    fn get_contract_transactions(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError> {
        dispatch!(self, get_contract_transactions, contract_id, limit)
    }
    fn put_account(&self, address: &PublicKey, account: &Account) -> Result<(), StorageError> {
        dispatch!(self, put_account, address, account)
    }
    fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, StorageError> {
        dispatch!(self, get_account, address)
    }
    fn delete_account(&self, address: &PublicKey) -> Result<(), StorageError> {
        dispatch!(self, delete_account, address)
    }
    fn get_all_accounts(&self) -> Result<Vec<(PublicKey, Account)>, StorageError> {
        dispatch!(self, get_all_accounts)
    }
    fn put_state_node(
        &self,
        level: u16,
        path: &[u8; 32],
        hash: &[u8; 32],
    ) -> Result<(), StorageError> {
        dispatch!(self, put_state_node, level, path, hash)
    }
    fn get_state_node(
        &self,
        level: u16,
        path: &[u8; 32],
    ) -> Result<Option<[u8; 32]>, StorageError> {
        dispatch!(self, get_state_node, level, path)
    }
    fn put_chain_state(&self, state: &ChainState) -> Result<(), StorageError> {
        dispatch!(self, put_chain_state, state)
    }
    fn get_chain_state(&self) -> Result<Option<ChainState>, StorageError> {
        dispatch!(self, get_chain_state)
    }
    fn put_contract_code(
        &self,
        contract_id: &ContractId,
        wasm_bytes: &[u8],
    ) -> Result<(), StorageError> {
        dispatch!(self, put_contract_code, contract_id, wasm_bytes)
    }
    fn get_contract_code(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError> {
        dispatch!(self, get_contract_code, contract_id)
    }
    fn put_contract_abi(&self, contract_id: &ContractId, abi: &[u8]) -> Result<(), StorageError> {
        dispatch!(self, put_contract_abi, contract_id, abi)
    }
    fn get_contract_abi(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError> {
        dispatch!(self, get_contract_abi, contract_id)
    }
    fn contract_storage_read(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, StorageError> {
        dispatch!(self, contract_storage_read, contract_id, key)
    }
    fn contract_storage_read_all(
        &self,
        contract_id: &ContractId,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
        dispatch!(self, contract_storage_read_all, contract_id)
    }
    fn contract_storage_write(
        &self,
        contract_id: &ContractId,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StorageError> {
        dispatch!(self, contract_storage_write, contract_id, key, value)
    }
    fn contract_storage_remove(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<(), StorageError> {
        dispatch!(self, contract_storage_remove, contract_id, key)
    }
    fn contract_emit_event(
        &self,
        contract_id: &ContractId,
        topic: &[u8],
        data: &[u8],
    ) -> Result<(), StorageError> {
        dispatch!(self, contract_emit_event, contract_id, topic, data)
    }
    fn get_contract_events(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<ContractEvents, StorageError> {
        dispatch!(self, get_contract_events, contract_id, limit)
    }
    fn get_all_contract_storage_keys(
        &self,
        contract_id: &ContractId,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        dispatch!(self, get_all_contract_storage_keys, contract_id)
    }
    fn put_contract_deployer(
        &self,
        contract_id: &ContractId,
        deployer: &PublicKey,
    ) -> Result<(), StorageError> {
        dispatch!(self, put_contract_deployer, contract_id, deployer)
    }
    fn get_contract_deployer(
        &self,
        contract_id: &ContractId,
    ) -> Result<Option<PublicKey>, StorageError> {
        dispatch!(self, get_contract_deployer, contract_id)
    }
    fn apply_batch(&self, batch: StorageBatch) -> Result<(), StorageError> {
        dispatch!(self, apply_batch, batch)
    }
    fn compact(&self) -> Result<(), StorageError> {
        dispatch!(self, compact)
    }
    fn get_storage_stats(&self) -> Result<StorageStats, StorageError> {
        dispatch!(self, get_storage_stats)
    }
    fn clear_pending_transactions(&self) -> Result<(), StorageError> {
        dispatch!(self, clear_pending_transactions)
    }
    fn backup_to(&self, path: &std::path::Path) -> Result<(), StorageError> {
        dispatch!(self, backup_to, path)
    }
    fn restore_from(&self, path: &std::path::Path) -> Result<(), StorageError> {
        dispatch!(self, restore_from, path)
    }
    fn backup_incremental(&self, base_path: &std::path::Path, output_path: &std::path::Path) -> Result<(), StorageError> {
        dispatch!(self, backup_incremental, base_path, output_path)
    }
    fn get_backup_manifest(&self, path: &std::path::Path) -> Result<Option<BackupManifest>, StorageError> {
        dispatch!(self, get_backup_manifest, path)
    }
    fn get_storage_metadata(&self, key: &str) -> Result<Option<String>, StorageError> {
        dispatch!(self, get_storage_metadata, key)
    }
    fn set_storage_metadata(&self, key: &str, value: &str) -> Result<(), StorageError> {
        dispatch!(self, set_storage_metadata, key, value)
    }
}
