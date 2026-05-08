use bincode;
use hex;
use sled::{Db, Tree};
use std::path::Path;
use thiserror::Error;

use crate::types::PublicKey;
use crate::types::{Account, Block, ChainState, ContractId, CryptoError, Transaction};

pub type ContractEvents = Vec<(Vec<u8>, Vec<u8>)>;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Sled error: {0}")]
    Sled(#[from] sled::Error),
    #[error("Bincode error: {0}")]
    Bincode(#[from] Box<bincode::ErrorKind>),
    #[error("Crypto error: {0}")]
    Crypto(#[from] CryptoError),
    #[error("Data not found")]
    NotFound,
    #[error("Transaction error: {0}")]
    Transaction(#[from] sled::transaction::TransactionError),
    #[error("Index error: {0}")]
    IndexError(String),
    #[error("Migration error: {0}")]
    MigrationError(String),
}

pub const CURRENT_STORAGE_FORMAT_VERSION: u32 = 1;
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

pub type StorageResult<T> = Result<T, StorageError>;
pub type KVList = Vec<(Vec<u8>, Vec<u8>)>;

pub trait Storage: Send + Sync {
    fn put_block(&self, block: &Block) -> Result<(), StorageError>;
    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StorageError>;
    fn get_latest_block(&self) -> Result<Option<Block>, StorageError>;
    fn get_chain_height(&self) -> Result<u64, StorageError>;
    fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError>;

    // Transaction Management
    fn put_transaction(&self, tx: &Transaction) -> Result<(), StorageError>;
    fn get_transaction(&self, tx_hash: &[u8; 32]) -> Result<Option<Transaction>, StorageError>;
    fn get_pending_transactions(&self) -> Result<Vec<Transaction>, StorageError>;
    fn put_pending_transaction(&self, tx: &Transaction) -> Result<(), StorageError>;
    fn remove_pending_transaction(&self, tx_hash: &[u8; 32]) -> Result<(), StorageError>;

    // Enhanced Transaction indexing for fast lookup
    fn index_transaction(
        &self,
        tx_hash: &[u8; 32],
        block_hash: &[u8; 32],
        tx_index_in_block: u32,
    ) -> Result<(), StorageError>;
    fn get_transaction_by_id(
        &self,
        tx_hash: &[u8; 32],
    ) -> Result<Option<(Block, Transaction)>, StorageError>;
    fn get_transactions_by_block(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<Vec<Transaction>, StorageError>;

    // New: Advanced indexing methods for Phase 3
    fn get_transactions_by_address(
        &self,
        address: &PublicKey,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError>;
    fn get_transactions_by_height_range(
        &self,
        from_height: u64,
        to_height: u64,
    ) -> Result<Vec<Transaction>, StorageError>;
    fn get_block_hashes_by_height_range(
        &self,
        from_height: u64,
        to_height: u64,
    ) -> Result<Vec<[u8; 32]>, StorageError>;
    fn get_account_transaction_count(&self, address: &PublicKey) -> Result<u64, StorageError>;
    fn get_contract_transactions(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError>;

    // Account State
    fn put_account(&self, address: &PublicKey, account: &Account) -> Result<(), StorageError>;
    fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, StorageError>;
    fn delete_account(&self, address: &PublicKey) -> Result<(), StorageError>;
    fn get_all_accounts(&self) -> Result<Vec<(PublicKey, Account)>, StorageError>;

    // State Tree Nodes (Incremental SMT)
    fn put_state_node(&self, level: u16, path: &[u8; 32], hash: &[u8; 32]) -> Result<(), StorageError>;
    fn get_state_node(&self, level: u16, path: &[u8; 32]) -> Result<Option<[u8; 32]>, StorageError>;

    // Global Chain State (used by Runtime/Ledger)
    fn put_chain_state(&self, state: &ChainState) -> Result<(), StorageError>;
    fn get_chain_state(&self) -> Result<Option<ChainState>, StorageError>;

    // Contract Code & State (used by ContractEngine)
    fn put_contract_code(
        &self,
        contract_id: &ContractId,
        wasm_bytes: &[u8],
    ) -> Result<(), StorageError>;
    fn get_contract_code(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError>;
    fn contract_storage_read(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, StorageError>;
    fn contract_storage_read_all(&self, contract_id: &ContractId) -> StorageResult<KVList>;
    fn contract_storage_write(
        &self,
        contract_id: &ContractId,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StorageError>;
    fn contract_storage_remove(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<(), StorageError>;
    fn contract_emit_event(
        &self,
        contract_id: &ContractId,
        topic: &[u8],
        data: &[u8],
    ) -> Result<(), StorageError>;
    fn get_contract_events(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<ContractEvents, StorageError>;
    fn get_all_contract_storage_keys(
        &self,
        contract_id: &ContractId,
    ) -> Result<Vec<Vec<u8>>, StorageError>;

    fn put_contract_deployer(
        &self,
        contract_id: &ContractId,
        deployer: &PublicKey,
    ) -> Result<(), StorageError>;
    fn get_contract_deployer(
        &self,
        contract_id: &ContractId,
    ) -> Result<Option<PublicKey>, StorageError>;

    // Atomic Batching for Block Application
    fn apply_batch(&self, batch: StorageBatch) -> Result<(), StorageError>;

    // New: Performance and maintenance methods
    fn compact(&self) -> Result<(), StorageError>;
    fn get_storage_stats(&self) -> Result<StorageStats, StorageError>;
    fn clear_pending_transactions(&self) -> Result<(), StorageError>;

    fn clone_storage(&self) -> Box<dyn Storage>;

    // Backup and restore
    fn backup_to(&self, path: &std::path::Path) -> Result<(), StorageError>;
    fn restore_from(&self, path: &std::path::Path) -> Result<(), StorageError>;

    // Storage metadata
    fn get_storage_metadata(&self, key: &str) -> Result<Option<String>, StorageError>;
    fn set_storage_metadata(&self, key: &str, value: &str) -> Result<(), StorageError>;

    fn storage_format_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .get_storage_metadata("storage_format_version")?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1))
    }

    fn schema_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .get_storage_metadata("schema_version")?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1))
    }

    fn validate_storage_version(&self, expected_format: u32) -> Result<(), StorageError> {
        let version = self.storage_format_version()?;
        if version != expected_format {
            return Err(StorageError::MigrationError(format!(
                "Storage format version mismatch: expected {}, found {}. Run migration.",
                expected_format, version
            )));
        }
        Ok(())
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct StorageBatch {
    pub ops: Vec<StorageOperation>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum StorageOperation {
    PutBlock(Vec<u8>, Vec<u8>),
    PutAccount(Vec<u8>, Vec<u8>),
    PutChainState(Vec<u8>, Vec<u8>),
    PutTransaction(Vec<u8>, Vec<u8>),
    PutTxIndex(Vec<u8>, Vec<u8>),
    PutContractCode(Vec<u8>, Vec<u8>),
    PutContractStorage(Vec<u8>, Vec<u8>),
    PutMempool(Vec<u8>, Vec<u8>),
    DeleteMempool(Vec<u8>),
    DeleteTransaction(Vec<u8>),
    PutStateNode(Vec<u8>, Vec<u8>),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StorageStats {
    pub total_blocks: u64,
    pub total_transactions: u64,
    pub total_accounts: u64,
    pub total_contracts: u64,
    pub mempool_size: u64,
    pub storage_size_bytes: u64,
    pub index_count: u64,
}

pub struct SledStorage {
    db: Db,
    meta_tree: Tree,
    blocks_tree: Tree,
    transactions_tree: Tree,
    mempool_tree: Tree,
    accounts_tree: Tree,
    contract_code_tree: Tree,
    contract_storage_tree: Tree,
    chain_state_tree: Tree,
    tx_by_block_tree: Tree,
    tx_to_block_tree: Tree,
    // New: Advanced indexing trees for Phase 3
    height_to_block_tree: Tree,
    address_to_tx_tree: Tree,
    contract_to_tx_tree: Tree,
    tx_count_tree: Tree,
    state_tree: Tree,
}

impl SledStorage {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::new_with_config(path, 64)
    }

    pub fn new_with_config(
        path: impl AsRef<Path>,
        cache_capacity_mb: u64,
    ) -> Result<Self, StorageError> {
        let config =
            sled::Config::default().path(path).cache_capacity(cache_capacity_mb * 1024 * 1024);
        let db = config.open()?;
        let storage = Self {
            meta_tree: db.open_tree("meta")?,
            blocks_tree: db.open_tree("blocks")?,
            transactions_tree: db.open_tree("transactions")?,
            mempool_tree: db.open_tree("mempool")?,
            accounts_tree: db.open_tree("accounts")?,
            contract_code_tree: db.open_tree("contract_code")?,
            contract_storage_tree: db.open_tree("contract_storage")?,
            chain_state_tree: db.open_tree("chain_state")?,
            tx_by_block_tree: db.open_tree("tx_by_block")?,
            tx_to_block_tree: db.open_tree("tx_to_block")?,
            height_to_block_tree: db.open_tree("height_to_block")?,
            address_to_tx_tree: db.open_tree("address_to_tx")?,
            contract_to_tx_tree: db.open_tree("contract_to_tx")?,
            tx_count_tree: db.open_tree("tx_count")?,
            state_tree: db.open_tree("state_nodes")?,
            db,
        };

        storage.initialize_metadata()?;

        // Recover any pending batches from previous crash
        storage.recover_pending_batches()?;

        // Migrate keys from old format (raw bytes) to prefixed format (hash:/acc:/code:)
        storage.migrate_key_prefixes()?;

        // Recover from interrupted compaction
        storage.recover_compaction()?;

        Ok(storage)
    }

    fn migrate_key_prefixes(&self) -> Result<(), StorageError> {
        let migration_flag = b"key_migration_complete";

        // Check if migration was already completed
        if self.meta_tree.get(migration_flag)?.is_some() {
            return Ok(());
        }

        // Phase 1: Write new prefixed keys alongside old raw keys
        self.migrate_tree_keys(&self.blocks_tree, b"hash:", 32)?;
        self.migrate_tree_keys(&self.accounts_tree, b"acc:", 32)?;
        self.migrate_tree_keys(&self.contract_code_tree, b"code:", 32)?;

        // Phase 2: Remove old raw keys
        self.cleanup_old_raw_keys(&self.blocks_tree, 32)?;
        self.cleanup_old_raw_keys(&self.accounts_tree, 32)?;
        self.cleanup_old_raw_keys(&self.contract_code_tree, 32)?;

        // Mark migration complete
        self.meta_tree.insert(migration_flag, b"1")?;
        self.meta_tree.flush()?;

        Ok(())
    }

    fn migrate_tree_keys(
        &self,
        tree: &sled::Tree,
        prefix: &[u8],
        raw_key_len: usize,
    ) -> Result<(), StorageError> {
        if let Some(Ok((first_key, _))) = tree.iter().next() {
            if first_key.len() == raw_key_len {
                for item in tree.iter() {
                    let (key, value) = item?;
                    if key.len() != raw_key_len {
                        continue;
                    }
                    let mut new_key = prefix.to_vec();
                    new_key.extend_from_slice(&key);
                    // Only write if not already present (crash recovery)
                    if tree.get(&new_key)?.is_none() {
                        tree.insert(new_key, value.as_ref())?;
                    }
                }
                tree.flush()?;
            }
        }
        Ok(())
    }

    fn cleanup_old_raw_keys(
        &self,
        tree: &sled::Tree,
        raw_key_len: usize,
    ) -> Result<(), StorageError> {
        let keys_to_remove: Vec<sled::IVec> = tree
            .iter()
            .filter_map(|r| r.ok())
            .filter(|(k, _)| k.len() == raw_key_len)
            .map(|(k, _)| k)
            .collect();
        for key in keys_to_remove {
            tree.remove(key)?;
        }
        tree.flush()?;
        Ok(())
    }

    fn recover_pending_batches(&self) -> Result<(), StorageError> {
        for item in self.db.scan_prefix("batch:") {
            let (key, value) = item?;
            if let Ok(batch) = bincode::deserialize::<StorageBatch>(&value) {
                // Replay the batch (WAL entry will be cleaned up on success)
                let result = self.apply_batch_without_wal(batch);
                if result.is_ok() {
                    self.db.remove(key)?;
                }
            } else {
                // Corrupted entry, remove it
                self.db.remove(key)?;
            }
        }
        self.db.flush()?;
        Ok(())
    }

    fn apply_batch_without_wal(&self, batch: StorageBatch) -> Result<(), StorageError> {
        for op in batch.ops {
            match op {
                StorageOperation::PutBlock(_, value) => {
                    if let Ok(block) = bincode::deserialize::<Block>(&value) {
                        self.put_block(&block)?;
                    }
                }
                StorageOperation::PutAccount(key, value) => {
                    let mut prefixed_key = b"acc:".to_vec();
                    prefixed_key.extend_from_slice(&key);
                    self.accounts_tree.insert(prefixed_key, value)?;
                }
                StorageOperation::PutChainState(key, value) => {
                    self.chain_state_tree.insert(key, value)?;
                }
                StorageOperation::PutTransaction(_key, value) => {
                    if let Ok(tx) = bincode::deserialize::<Transaction>(&value) {
                        self.put_transaction(&tx)?;
                    }
                }
                StorageOperation::PutStateNode(key, value) => {
                    self.state_tree.insert(key, value)?;
                }
                StorageOperation::PutTxIndex(key, value) => {
                    self.tx_by_block_tree.insert(key, value)?;
                }
                StorageOperation::PutContractCode(key, value) => {
                    let mut prefixed_key = b"code:".to_vec();
                    prefixed_key.extend_from_slice(&key);
                    self.contract_code_tree.insert(prefixed_key, value)?;
                }
                StorageOperation::PutContractStorage(key, value) => {
                    self.contract_storage_tree.insert(key, value)?;
                }
                StorageOperation::PutMempool(key, value) => {
                    self.mempool_tree.insert(key, value)?;
                }
                StorageOperation::DeleteMempool(key) => {
                    self.mempool_tree.remove(key)?;
                }
                StorageOperation::DeleteTransaction(key) => {
                    self.transactions_tree.remove(key)?;
                }
            }
        }
        self.blocks_tree.flush()?;
        self.accounts_tree.flush()?;
        self.chain_state_tree.flush()?;
        self.transactions_tree.flush()?;
        self.tx_by_block_tree.flush()?;
        self.contract_code_tree.flush()?;
        self.contract_storage_tree.flush()?;
        self.mempool_tree.flush()?;
        Ok(())
    }

    // Helper methods for advanced indexing
    fn index_transaction_by_address(
        &self,
        address: &PublicKey,
        tx_hash: &[u8; 32],
        timestamp: u64,
    ) -> Result<(), StorageError> {
        let key = format!(
            "{}:{:0>20}:{}",
            hex::encode(address.to_bytes()),
            timestamp,
            hex::encode(tx_hash)
        );
        self.address_to_tx_tree.insert(key.as_bytes(), tx_hash)?;
        Ok(())
    }

    fn index_transaction_by_contract(
        &self,
        contract_id: &ContractId,
        tx_hash: &[u8; 32],
        timestamp: u64,
    ) -> Result<(), StorageError> {
        let key =
            format!("{}:{:0>20}:{}", hex::encode(contract_id.id), timestamp, hex::encode(tx_hash));
        self.contract_to_tx_tree.insert(key.as_bytes(), tx_hash)?;
        Ok(())
    }

    fn increment_transaction_count(&self, address: &PublicKey) -> Result<(), StorageError> {
        let key = hex::encode(address.to_bytes());
        let current_count = self
            .tx_count_tree
            .get(key.as_bytes())?
            .map(|v| String::from_utf8_lossy(&v).parse::<u64>().unwrap_or(0))
            .unwrap_or(0);
        let new_count = current_count + 1;
        self.tx_count_tree.insert(key.as_bytes(), new_count.to_string().as_bytes())?;
        Ok(())
    }

    fn compact_tree(tree: &sled::Tree) -> Result<(), StorageError> {
        const BATCH_SIZE: usize = 10_000;
        let mut batch = std::collections::BTreeMap::<Vec<u8>, Vec<u8>>::new();
        let mut total = 0usize;
        for item in tree.iter() {
            let (key, value) = item?;
            batch.insert(key.to_vec(), value.to_vec());
            total += 1;
            if batch.len() >= BATCH_SIZE {
                for (k, v) in std::mem::take(&mut batch) {
                    tree.insert(k, v)?;
                }
                tree.flush()?;
            }
        }
        // Flush remaining entries
        for (key, value) in batch {
            tree.insert(key, value)?;
        }
        if total > 0 {
            tree.flush()?;
        }
        Ok(())
    }

    fn initialize_metadata(&self) -> Result<(), StorageError> {
        if self.meta_tree.get("storage_format_version")?.is_none() {
            self.meta_tree.insert(
                "storage_format_version",
                CURRENT_STORAGE_FORMAT_VERSION.to_string().as_bytes(),
            )?;
        }
        if self.meta_tree.get("schema_version")?.is_none() {
            self.meta_tree
                .insert("schema_version", CURRENT_SCHEMA_VERSION.to_string().as_bytes())?;
        }
        if self.meta_tree.get("created_with_baals_version")?.is_none() {
            self.meta_tree
                .insert("created_with_baals_version", env!("CARGO_PKG_VERSION").as_bytes())?;
        }
        self.meta_tree.flush()?;
        Ok(())
    }

    fn recover_compaction(&self) -> Result<(), StorageError> {
        if self.db.get("compaction_in_progress")?.is_none() {
            return Ok(());
        }

        log::warn!("Detected incomplete compaction, recovering...");

        Self::compact_tree(&self.blocks_tree)?;
        Self::compact_tree(&self.transactions_tree)?;
        Self::compact_tree(&self.mempool_tree)?;
        Self::compact_tree(&self.accounts_tree)?;
        Self::compact_tree(&self.contract_code_tree)?;
        Self::compact_tree(&self.contract_storage_tree)?;
        Self::compact_tree(&self.chain_state_tree)?;
        Self::compact_tree(&self.tx_by_block_tree)?;
        Self::compact_tree(&self.tx_to_block_tree)?;
        Self::compact_tree(&self.height_to_block_tree)?;
        Self::compact_tree(&self.address_to_tx_tree)?;
        Self::compact_tree(&self.contract_to_tx_tree)?;
        Self::compact_tree(&self.tx_count_tree)?;

        self.db.remove("compaction_in_progress")?;
        self.db.flush()?;

        log::info!("Compaction recovery completed");
        Ok(())
    }
}

impl Clone for SledStorage {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            meta_tree: self.meta_tree.clone(),
            blocks_tree: self.blocks_tree.clone(),
            transactions_tree: self.transactions_tree.clone(),
            mempool_tree: self.mempool_tree.clone(),
            accounts_tree: self.accounts_tree.clone(),
            contract_code_tree: self.contract_code_tree.clone(),
            contract_storage_tree: self.contract_storage_tree.clone(),
            chain_state_tree: self.chain_state_tree.clone(),
            tx_by_block_tree: self.tx_by_block_tree.clone(),
            tx_to_block_tree: self.tx_to_block_tree.clone(),
            // New: Advanced indexing trees
            height_to_block_tree: self.height_to_block_tree.clone(),
            address_to_tx_tree: self.address_to_tx_tree.clone(),
            contract_to_tx_tree: self.contract_to_tx_tree.clone(),
            tx_count_tree: self.tx_count_tree.clone(),
            state_tree: self.state_tree.clone(),
        }
    }
}

impl Storage for SledStorage {
    fn clone_storage(&self) -> Box<dyn Storage> {
        Box::new(self.clone())
    }
    fn put_block(&self, block: &Block) -> Result<(), StorageError> {
        let block_hash = block.hash;
        let block_height = block.index;
        let encoded = bincode::serialize(block)?;

        let mut key = b"hash:".to_vec();
        key.extend_from_slice(&block_hash);
        self.blocks_tree.insert(key, encoded.clone())?;
        self.blocks_tree.insert(format!("height:{:0>20}", block_height).as_bytes(), encoded)?;
        Ok(())
    }

    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StorageError> {
        let mut key = b"hash:".to_vec();
        key.extend_from_slice(hash);
        let encoded = self.blocks_tree.get(key)?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn get_latest_block(&self) -> Result<Option<Block>, StorageError> {
        let mut iter = self.blocks_tree.scan_prefix("height:").rev();
        if let Some(Ok((_key, encoded))) = iter.next() {
            Ok(Some(bincode::deserialize(&encoded)?))
        } else {
            Ok(None)
        }
    }

    fn get_chain_height(&self) -> Result<u64, StorageError> {
        Ok(self.get_latest_block()?.map_or(0, |b| b.index))
    }

    fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError> {
        let encoded = self.blocks_tree.get(format!("height:{:0>20}", height).as_bytes())?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn put_transaction(&self, tx: &Transaction) -> Result<(), StorageError> {
        let encoded = bincode::serialize(tx)?;
        self.transactions_tree.insert(tx.hash, encoded)?;

        // Index transaction by sender address
        self.index_transaction_by_address(&tx.sender, &tx.hash, tx.timestamp)?;

        // If this is a contract-related transaction, index by contract
        if let crate::types::Address::Contract(contract_id) = &tx.recipient {
            self.index_transaction_by_contract(contract_id, &tx.hash, tx.timestamp)?;
        }

        // Increment transaction count for sender
        self.increment_transaction_count(&tx.sender)?;

        Ok(())
    }

    fn get_transaction(&self, tx_hash: &[u8; 32]) -> Result<Option<Transaction>, StorageError> {
        let encoded = self.transactions_tree.get(tx_hash)?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn get_pending_transactions(&self) -> Result<Vec<Transaction>, StorageError> {
        let mut transactions = Vec::new();
        for item in self.mempool_tree.scan_prefix("pending:") {
            let (_key, encoded) = item?;
            transactions.push(bincode::deserialize(&encoded)?);
        }
        Ok(transactions)
    }

    fn put_pending_transaction(&self, tx: &Transaction) -> Result<(), StorageError> {
        let key = format!("pending:{}", hex::encode(tx.hash));
        let encoded = bincode::serialize(tx)?;
        self.mempool_tree.insert(key.as_bytes(), encoded)?;
        Ok(())
    }

    fn remove_pending_transaction(&self, tx_hash: &[u8; 32]) -> Result<(), StorageError> {
        let key = format!("pending:{}", hex::encode(tx_hash));
        self.mempool_tree.remove(key.as_bytes())?;
        Ok(())
    }

    // New: Transaction indexing for fast lookup by block
    fn index_transaction(
        &self,
        tx_hash: &[u8; 32],
        block_hash: &[u8; 32],
        tx_index_in_block: u32,
    ) -> Result<(), StorageError> {
        let key = format!(
            "block_tx:{}:{}:{:0>10}",
            hex::encode(block_hash),
            hex::encode(tx_hash),
            tx_index_in_block
        );
        self.tx_by_block_tree.insert(key, tx_hash.as_slice())?;
        // Store reverse mapping: tx_hash → block_hash
        self.tx_to_block_tree.insert(tx_hash.as_slice(), block_hash.as_slice())?;
        Ok(())
    }

    fn get_transaction_by_id(
        &self,
        tx_hash: &[u8; 32],
    ) -> Result<Option<(Block, Transaction)>, StorageError> {
        // Look up block hash for this transaction
        let block_hash = match self.tx_to_block_tree.get(tx_hash)? {
            Some(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                arr
            }
            _ => return Ok(None),
        };
        let tx = match self.get_transaction(tx_hash)? {
            Some(t) => t,
            None => return Ok(None),
        };
        let block = match self.get_block(&block_hash)? {
            Some(b) => b,
            None => return Ok(None),
        };
        Ok(Some((block, tx)))
    }

    fn get_transactions_by_block(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<Vec<Transaction>, StorageError> {
        let mut indexed: Vec<(u32, Transaction)> = Vec::new();
        let prefix_string = format!("block_tx:{}:", hex::encode(block_hash));
        for item in self.tx_by_block_tree.scan_prefix(prefix_string.as_bytes()) {
            let (key, tx_hash_bytes) = item?;
            let tx_hash_array: [u8; 32] =
                tx_hash_bytes.as_ref().try_into().map_err(|_| CryptoError::HashConversionError)?;
            if let Some(tx) = self.get_transaction(&tx_hash_array)? {
                // Extract index from key: "block_tx:{hash}:{tx_hash}:{index}"
                let idx = std::str::from_utf8(&key)
                    .ok()
                    .and_then(|s| s.rsplit(':').next())
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(u32::MAX);
                indexed.push((idx, tx));
            }
        }
        indexed.sort_by_key(|(idx, _)| *idx);
        Ok(indexed.into_iter().map(|(_, tx)| tx).collect())
    }

    // New: Advanced indexing methods for Phase 3
    fn get_transactions_by_address(
        &self,
        address: &PublicKey,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut transactions = Vec::new();
        let prefix_string = format!("{}:", hex::encode(address.to_bytes()));
        for item in self.address_to_tx_tree.scan_prefix(prefix_string.as_bytes()).rev() {
            let (_key, tx_hash_bytes) = item?;
            let tx_hash_array: [u8; 32] =
                tx_hash_bytes.as_ref().try_into().map_err(|_| CryptoError::HashConversionError)?;
            if let Some(tx) = self.get_transaction(&tx_hash_array)? {
                transactions.push(tx);
                if transactions.len() >= limit {
                    break;
                }
            }
        }
        Ok(transactions)
    }

    fn get_transactions_by_height_range(
        &self,
        from_height: u64,
        to_height: u64,
    ) -> Result<Vec<Transaction>, StorageError> {
        let mut transactions = Vec::new();
        let mut iter = self.blocks_tree.scan_prefix(b"height:");
        while let Some(Ok((_key, encoded))) = iter.next() {
            let block: Block = bincode::deserialize(&encoded)?;
            if block.index >= from_height && block.index <= to_height {
                for item in self
                    .tx_by_block_tree
                    .scan_prefix(format!("block_tx:{}:", hex::encode(block.hash)).as_bytes())
                {
                    let (_key, tx_hash_bytes) = item?;
                    let tx_hash_array: [u8; 32] = tx_hash_bytes
                        .as_ref()
                        .try_into()
                        .map_err(|_| CryptoError::HashConversionError)?;
                    if let Some(tx) = self.get_transaction(&tx_hash_array)? {
                        transactions.push(tx);
                    }
                }
            }
        }
        Ok(transactions)
    }

    fn get_block_hashes_by_height_range(
        &self,
        from_height: u64,
        to_height: u64,
    ) -> Result<Vec<[u8; 32]>, StorageError> {
        let mut block_hashes = Vec::new();
        let mut iter = self.blocks_tree.scan_prefix(b"height:");
        while let Some(Ok((_key, encoded))) = iter.next() {
            let block: Block = bincode::deserialize(&encoded)?;
            if block.index >= from_height && block.index <= to_height {
                block_hashes.push(block.hash);
            }
        }
        Ok(block_hashes)
    }

    fn get_account_transaction_count(&self, address: &PublicKey) -> Result<u64, StorageError> {
        let key = address.to_bytes();
        Ok(self.tx_count_tree.get(key)?.and_then(|v| {
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&v);
            Some(u64::from_le_bytes(arr))
        }).unwrap_or(0))
    }

    fn put_state_node(&self, level: u16, path: &[u8; 32], hash: &[u8; 32]) -> Result<(), StorageError> {
        let mut key = level.to_be_bytes().to_vec();
        key.extend_from_slice(path);
        self.state_tree.insert(key, hash)?;
        Ok(())
    }

    fn get_state_node(&self, level: u16, path: &[u8; 32]) -> Result<Option<[u8; 32]>, StorageError> {
        let mut key = level.to_be_bytes().to_vec();
        key.extend_from_slice(path);
        let val = self.state_tree.get(key)?;
        Ok(val.map(|v| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&v);
            arr
        }))
    }

    fn get_contract_transactions(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut transactions = Vec::new();
        let prefix_string = format!("{}:", hex::encode(contract_id.id));
        for item in self.contract_to_tx_tree.scan_prefix(prefix_string.as_bytes()).rev() {
            let (_key, tx_hash_bytes) = item?;
            let tx_hash_array: [u8; 32] =
                tx_hash_bytes.as_ref().try_into().map_err(|_| CryptoError::HashConversionError)?;
            if let Some(tx) = self.get_transaction(&tx_hash_array)? {
                transactions.push(tx);
                if transactions.len() >= limit {
                    break;
                }
            }
        }
        Ok(transactions)
    }

    fn put_account(&self, address: &PublicKey, account: &Account) -> Result<(), StorageError> {
        let encoded = bincode::serialize(account)?;
        let mut key = b"acc:".to_vec();
        key.extend_from_slice(&address.to_bytes());
        self.accounts_tree.insert(key, encoded)?;
        self.accounts_tree.flush()?;
        Ok(())
    }

    fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, StorageError> {
        let mut key = b"acc:".to_vec();
        key.extend_from_slice(&address.to_bytes());
        let encoded = self.accounts_tree.get(key)?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn delete_account(&self, address: &PublicKey) -> Result<(), StorageError> {
        let mut key = b"acc:".to_vec();
        key.extend_from_slice(&address.to_bytes());
        self.accounts_tree.remove(key)?;
        Ok(())
    }

    fn get_all_accounts(&self) -> Result<Vec<(PublicKey, Account)>, StorageError> {
        let mut accounts = Vec::new();
        for item in self.accounts_tree.scan_prefix("acc:") {
            let (key, value) = item?;
            let key_bytes = key.as_ref();
            if key_bytes.len() > 4 {
                let addr_bytes: [u8; 32] = key_bytes[4..]
                    .try_into()
                    .map_err(|_| StorageError::Crypto(CryptoError::InvalidPublicKey))?;
                let pk = PublicKey::from_bytes(&addr_bytes)?;
                let account: Account = bincode::deserialize(&value)?;
                accounts.push((pk, account));
            }
        }
        Ok(accounts)
    }

    fn put_chain_state(&self, state: &ChainState) -> Result<(), StorageError> {
        let encoded = bincode::serialize(state)?;
        self.chain_state_tree.insert("global:current", encoded)?;
        Ok(())
    }

    fn get_chain_state(&self) -> Result<Option<ChainState>, StorageError> {
        // Use the same raw string key that put_chain_state uses
        let result = self.chain_state_tree.get("global:current")?;
        match result {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    fn put_contract_code(
        &self,
        contract_id: &ContractId,
        wasm_bytes: &[u8],
    ) -> Result<(), StorageError> {
        let mut key = b"code:".to_vec();
        key.extend_from_slice(&contract_id.id);
        self.contract_code_tree.insert(key, wasm_bytes)?;
        Ok(())
    }

    fn get_contract_code(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError> {
        let mut key = b"code:".to_vec();
        key.extend_from_slice(&contract_id.id);
        let encoded = self.contract_code_tree.get(key)?;
        Ok(encoded.map(|e| e.to_vec()))
    }

    fn contract_storage_read(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let full_key = format!("state:{}:{}", hex::encode(contract_id.id), hex::encode(key));
        let encoded = self.contract_storage_tree.get(full_key)?;
        Ok(encoded.map(|e| e.to_vec()))
    }

    fn contract_storage_read_all(
        &self,
        contract_id: &ContractId,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
        let prefix = format!("state:{}:", hex::encode(contract_id.id));
        let mut kv = Vec::new();

        for item in self.contract_storage_tree.scan_prefix(prefix.as_bytes()) {
            let (raw_key, value) = item?;
            let raw_key_str = String::from_utf8(raw_key.to_vec())
                .map_err(|e| StorageError::IndexError(format!("Invalid UTF-8 key: {}", e)))?;
            let key_hex = raw_key_str
                .strip_prefix(&prefix)
                .ok_or_else(|| StorageError::IndexError("Invalid contract storage key".into()))?;
            let decoded_key = hex::decode(key_hex)
                .map_err(|e| StorageError::IndexError(format!("Invalid hex key: {}", e)))?;
            kv.push((decoded_key, value.to_vec()));
        }

        Ok(kv)
    }

    fn contract_storage_write(
        &self,
        contract_id: &ContractId,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StorageError> {
        let full_key = format!("state:{}:{}", hex::encode(contract_id.id), hex::encode(key));
        self.contract_storage_tree.insert(full_key, value)?;
        Ok(())
    }

    fn contract_storage_remove(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<(), StorageError> {
        let full_key = format!("state:{}:{}", hex::encode(contract_id.id), hex::encode(key));
        self.contract_storage_tree.remove(full_key)?;
        Ok(())
    }

    fn contract_emit_event(
        &self,
        contract_id: &ContractId,
        topic: &[u8],
        data: &[u8],
    ) -> Result<(), StorageError> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let key = format!(
            "event:{}:{:0>20}:{}",
            hex::encode(contract_id.id),
            timestamp,
            hex::encode(topic)
        );
        let value = data.to_vec();
        self.contract_storage_tree.insert(key, value)?;
        Ok(())
    }

    fn get_contract_events(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<ContractEvents, StorageError> {
        let prefix = format!("event:{}:", hex::encode(contract_id.id));
        let mut events = Vec::new();
        for item in self.contract_storage_tree.scan_prefix(prefix.as_bytes()) {
            let (raw_key, value) = item?;
            let key_str = String::from_utf8_lossy(&raw_key);
            let topic_hex = key_str.rsplit(':').next().unwrap_or("");
            let topic = hex::decode(topic_hex).unwrap_or_default();
            events.push((topic, value.to_vec()));
            if events.len() >= limit {
                break;
            }
        }
        Ok(events)
    }

    fn get_all_contract_storage_keys(
        &self,
        contract_id: &ContractId,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        let prefix = format!("state:{}:", hex::encode(contract_id.id));
        let mut keys = Vec::new();

        for item in self.contract_storage_tree.scan_prefix(prefix.as_bytes()) {
            let (raw_key, _value) = item?;
            let raw_key_str = String::from_utf8(raw_key.to_vec())
                .map_err(|e| StorageError::IndexError(format!("Invalid UTF-8 key: {}", e)))?;
            let key_hex = raw_key_str
                .strip_prefix(&prefix)
                .ok_or_else(|| StorageError::IndexError("Invalid contract storage key".into()))?;
            let decoded = hex::decode(key_hex)
                .map_err(|e| StorageError::IndexError(format!("Invalid hex key: {}", e)))?;
            keys.push(decoded);
        }

        Ok(keys)
    }

    fn put_contract_deployer(
        &self,
        contract_id: &ContractId,
        deployer: &PublicKey,
    ) -> Result<(), StorageError> {
        let mut key = b"deployer:".to_vec();
        key.extend_from_slice(&contract_id.id);
        let encoded = bincode::serialize(deployer)?;
        self.contract_code_tree.insert(key, encoded)?;
        Ok(())
    }

    fn get_contract_deployer(
        &self,
        contract_id: &ContractId,
    ) -> Result<Option<PublicKey>, StorageError> {
        let mut key = b"deployer:".to_vec();
        key.extend_from_slice(&contract_id.id);
        let encoded = self.contract_code_tree.get(key)?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn apply_batch(&self, batch: StorageBatch) -> Result<(), StorageError> {
        // Write-ahead log for crash recovery
        let batch_id = format!(
            "batch:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let batch_bytes = bincode::serialize(&batch)?;
        self.db.insert(batch_id.as_bytes(), batch_bytes)?;
        self.db.flush()?;

        let result = self.apply_batch_without_wal(batch);
        if result.is_ok() {
            self.db.remove(batch_id.as_bytes())?;
            self.db.flush()?;
        }
        result
    }

    // New: Performance and maintenance methods
    fn compact(&self) -> Result<(), StorageError> {
        // Write crash-recovery marker
        self.db.insert("compaction_in_progress", b"1")?;
        self.db.flush()?;

        Self::compact_tree(&self.blocks_tree)?;
        Self::compact_tree(&self.transactions_tree)?;
        Self::compact_tree(&self.mempool_tree)?;
        Self::compact_tree(&self.accounts_tree)?;
        Self::compact_tree(&self.contract_code_tree)?;
        Self::compact_tree(&self.contract_storage_tree)?;
        Self::compact_tree(&self.chain_state_tree)?;
        Self::compact_tree(&self.tx_by_block_tree)?;
        Self::compact_tree(&self.tx_to_block_tree)?;
        Self::compact_tree(&self.height_to_block_tree)?;
        Self::compact_tree(&self.address_to_tx_tree)?;
        Self::compact_tree(&self.contract_to_tx_tree)?;
        Self::compact_tree(&self.tx_count_tree)?;

        // Remove crash-recovery marker
        self.db.remove("compaction_in_progress")?;
        self.db.flush()?;

        Ok(())
    }

    fn backup_to(&self, path: &std::path::Path) -> Result<(), StorageError> {
        use std::fs;
        // Create backup directory
        fs::create_dir_all(path).map_err(|e| StorageError::IndexError(e.to_string()))?;
        let backup_db = sled::open(path).map_err(StorageError::Sled)?;

        // Export all data trees
        let export_tree = |src: &sled::Tree, dst_tree: &sled::Tree| -> Result<(), StorageError> {
            for item in src.iter() {
                let (key, value) = item?;
                dst_tree.insert(key, value)?;
            }
            dst_tree.flush()?;
            Ok(())
        };

        export_tree(&self.blocks_tree, &backup_db.open_tree("blocks")?)?;
        export_tree(&self.transactions_tree, &backup_db.open_tree("transactions")?)?;
        export_tree(&self.mempool_tree, &backup_db.open_tree("mempool")?)?;
        export_tree(&self.accounts_tree, &backup_db.open_tree("accounts")?)?;
        export_tree(&self.chain_state_tree, &backup_db.open_tree("chain_state")?)?;
        export_tree(&self.contract_code_tree, &backup_db.open_tree("contract_code")?)?;
        export_tree(&self.contract_storage_tree, &backup_db.open_tree("contract_storage")?)?;
        export_tree(&self.height_to_block_tree, &backup_db.open_tree("height_to_block")?)?;
        export_tree(&self.tx_by_block_tree, &backup_db.open_tree("tx_by_block")?)?;
        export_tree(&self.tx_to_block_tree, &backup_db.open_tree("tx_to_block")?)?;
        export_tree(&self.address_to_tx_tree, &backup_db.open_tree("address_to_tx")?)?;
        export_tree(&self.contract_to_tx_tree, &backup_db.open_tree("contract_to_tx")?)?;
        export_tree(&self.tx_count_tree, &backup_db.open_tree("tx_count")?)?;

        backup_db.flush()?;
        log::info!("Storage backup completed to {:?}", path);
        Ok(())
    }

    fn restore_from(&self, path: &std::path::Path) -> Result<(), StorageError> {
        let backup_db = sled::open(path).map_err(StorageError::Sled)?;

        let restore_tree = |src_tree: &sled::Tree, dst: &sled::Tree| -> Result<(), StorageError> {
            dst.clear()?;
            for item in src_tree.iter() {
                let (key, value) = item?;
                dst.insert(key, value)?;
            }
            dst.flush()?;
            Ok(())
        };

        restore_tree(&backup_db.open_tree("blocks")?, &self.blocks_tree)?;
        restore_tree(&backup_db.open_tree("transactions")?, &self.transactions_tree)?;
        restore_tree(&backup_db.open_tree("mempool")?, &self.mempool_tree)?;
        restore_tree(&backup_db.open_tree("accounts")?, &self.accounts_tree)?;
        restore_tree(&backup_db.open_tree("chain_state")?, &self.chain_state_tree)?;
        restore_tree(&backup_db.open_tree("contract_code")?, &self.contract_code_tree)?;
        restore_tree(&backup_db.open_tree("contract_storage")?, &self.contract_storage_tree)?;
        restore_tree(&backup_db.open_tree("height_to_block")?, &self.height_to_block_tree)?;
        restore_tree(&backup_db.open_tree("tx_by_block")?, &self.tx_by_block_tree)?;
        restore_tree(&backup_db.open_tree("tx_to_block")?, &self.tx_to_block_tree)?;
        restore_tree(&backup_db.open_tree("address_to_tx")?, &self.address_to_tx_tree)?;
        restore_tree(&backup_db.open_tree("contract_to_tx")?, &self.contract_to_tx_tree)?;
        restore_tree(&backup_db.open_tree("tx_count")?, &self.tx_count_tree)?;

        self.db.flush()?;
        log::info!("Storage restore completed from {:?}", path);
        Ok(())
    }

    fn get_storage_stats(&self) -> Result<StorageStats, StorageError> {
        let total_blocks = self.blocks_tree.len() as u64;
        let total_transactions = self.transactions_tree.len() as u64;
        let total_accounts = self.accounts_tree.len() as u64;
        let total_contracts = self.contract_code_tree.len() as u64;
        let mempool_size = self.mempool_tree.len() as u64;
        let storage_size_bytes = self.db.size_on_disk()?;
        let index_count = (self.tx_by_block_tree.len()
            + self.address_to_tx_tree.len()
            + self.contract_to_tx_tree.len()
            + self.tx_count_tree.len()) as u64;

        Ok(StorageStats {
            total_blocks,
            total_transactions,
            total_accounts,
            total_contracts,
            mempool_size,
            storage_size_bytes,
            index_count,
        })
    }

    fn clear_pending_transactions(&self) -> Result<(), StorageError> {
        for item in self.mempool_tree.scan_prefix("pending:") {
            let (_key, _value) = item?;
            self.mempool_tree.remove(_key)?;
        }
        Ok(())
    }

    fn get_storage_metadata(&self, key: &str) -> Result<Option<String>, StorageError> {
        match self.meta_tree.get(key)? {
            Some(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).to_string())),
            None => Ok(None),
        }
    }

    fn set_storage_metadata(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.meta_tree.insert(key, value.as_bytes())?;
        self.meta_tree.flush()?;
        Ok(())
    }

    fn storage_format_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .get_storage_metadata("storage_format_version")?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1))
    }

    fn schema_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .get_storage_metadata("schema_version")?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1))
    }

    fn validate_storage_version(&self, expected_format: u32) -> Result<(), StorageError> {
        let version = self.storage_format_version()?;
        if version != expected_format {
            return Err(StorageError::MigrationError(format!(
                "Storage format version mismatch: expected {}, found {}. Run migration.",
                expected_format, version
            )));
        }
        Ok(())
    }
}
