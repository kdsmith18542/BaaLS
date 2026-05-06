use bincode;
use hex;
use sled::{Db, Tree};
use std::path::Path;
use thiserror::Error;

use crate::types::PublicKey;
use crate::types::{Account, Block, ChainState, ContractId, CryptoError, Transaction};

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Sled error: {0}")]
    Sled(#[from] sled::Error),
    #[error("Bincode error: {0}")]
    Bincode(#[from] Box<bincode::ErrorKind>),
    #[error("Crypto error: {0}")]
    Crypto(#[from] CryptoError),
    #[error("Postcard error: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("Data not found")]
    NotFound,
    #[error("Transaction error: {0}")]
    Transaction(#[from] sled::transaction::TransactionError),
    #[error("Index error: {0}")]
    IndexError(String),
}

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
    ) -> Result<Option<Transaction>, StorageError>;
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

    // Account State Management (used by Ledger)
    fn put_account(&self, address: &PublicKey, account: &Account) -> Result<(), StorageError>;
    fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, StorageError>;
    fn delete_account(&self, address: &PublicKey) -> Result<(), StorageError>;
    fn get_all_accounts(&self) -> Result<Vec<(PublicKey, Account)>, StorageError>;

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

    // Atomic Batching for Block Application
    fn apply_batch(&self, batch: StorageBatch) -> Result<(), StorageError>;

    // New: Performance and maintenance methods
    fn compact(&self) -> Result<(), StorageError>;
    fn get_storage_stats(&self) -> Result<StorageStats, StorageError>;
    fn clear_mempool(&self) -> Result<(), StorageError>;

    fn clone_storage(&self) -> Box<dyn Storage>;

    // Backup and restore
    fn backup_to(&self, path: &std::path::Path) -> Result<(), StorageError>;
    fn restore_from(&self, path: &std::path::Path) -> Result<(), StorageError>;
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
    blocks_tree: Tree,
    transactions_tree: Tree,
    mempool_tree: Tree,
    accounts_tree: Tree,
    contract_code_tree: Tree,
    contract_storage_tree: Tree,
    chain_state_tree: Tree,
    tx_by_block_tree: Tree,
    // New: Advanced indexing trees for Phase 3
    height_to_block_tree: Tree,
    address_to_tx_tree: Tree,
    contract_to_tx_tree: Tree,
    tx_count_tree: Tree,
}

impl SledStorage {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db = sled::open(path)?;
        let storage = Self {
            blocks_tree: db.open_tree("blocks")?,
            transactions_tree: db.open_tree("transactions")?,
            mempool_tree: db.open_tree("mempool")?,
            accounts_tree: db.open_tree("accounts")?,
            contract_code_tree: db.open_tree("contract_code")?,
            contract_storage_tree: db.open_tree("contract_storage")?,
            chain_state_tree: db.open_tree("chain_state")?,
            tx_by_block_tree: db.open_tree("tx_by_block")?,
            // New: Advanced indexing trees
            height_to_block_tree: db.open_tree("height_to_block")?,
            address_to_tx_tree: db.open_tree("address_to_tx")?,
            contract_to_tx_tree: db.open_tree("contract_to_tx")?,
            tx_count_tree: db.open_tree("tx_count")?,
            db,
        };

        // Recover any pending batches from previous crash
        storage.recover_pending_batches()?;

        Ok(storage)
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
                StorageOperation::PutBlock(key, value) => {
                    self.blocks_tree.insert(key, value)?;
                }
                StorageOperation::PutAccount(key, value) => {
                    self.accounts_tree.insert(key, value)?;
                }
                StorageOperation::PutChainState(key, value) => {
                    self.chain_state_tree.insert(key, value)?;
                }
                StorageOperation::PutTransaction(key, value) => {
                    self.transactions_tree.insert(key, value)?;
                }
                StorageOperation::PutTxIndex(key, value) => {
                    self.tx_by_block_tree.insert(key, value)?;
                }
                StorageOperation::PutContractCode(key, value) => {
                    self.contract_code_tree.insert(key, value)?;
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
        let key = format!(
            "{}:{:0>20}:{}",
            hex::encode(contract_id.id),
            timestamp,
            hex::encode(tx_hash)
        );
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
        self.tx_count_tree
            .insert(key.as_bytes(), new_count.to_string().as_bytes())?;
        Ok(())
    }
}

impl Clone for SledStorage {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            blocks_tree: self.blocks_tree.clone(),
            transactions_tree: self.transactions_tree.clone(),
            mempool_tree: self.mempool_tree.clone(),
            accounts_tree: self.accounts_tree.clone(),
            contract_code_tree: self.contract_code_tree.clone(),
            contract_storage_tree: self.contract_storage_tree.clone(),
            chain_state_tree: self.chain_state_tree.clone(),
            tx_by_block_tree: self.tx_by_block_tree.clone(),
            // New: Advanced indexing trees
            height_to_block_tree: self.height_to_block_tree.clone(),
            address_to_tx_tree: self.address_to_tx_tree.clone(),
            contract_to_tx_tree: self.contract_to_tx_tree.clone(),
            tx_count_tree: self.tx_count_tree.clone(),
        }
    }
}

impl Storage for SledStorage {
    fn put_block(&self, block: &Block) -> Result<(), StorageError> {
        let block_hash = block.hash;
        let block_height = block.index;
        let encoded = bincode::serialize(block)?;

        self.blocks_tree.insert(block_hash, encoded.clone())?;
        self.blocks_tree
            .insert(format!("height:{:0>20}", block_height).as_bytes(), encoded)?;
        Ok(())
    }

    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StorageError> {
        let encoded = self.blocks_tree.get(hash)?;
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
        let encoded = self
            .blocks_tree
            .get(format!("height:{:0>20}", height).as_bytes())?;
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

    fn remove_pending_transaction(&self, tx_hash: &[u8; 32]) -> Result<(), StorageError> {
        self.mempool_tree.remove(tx_hash)?;
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
        Ok(())
    }

    fn get_transaction_by_id(
        &self,
        tx_hash: &[u8; 32],
    ) -> Result<Option<Transaction>, StorageError> {
        let encoded = self.transactions_tree.get(tx_hash)?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn get_transactions_by_block(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<Vec<Transaction>, StorageError> {
        let mut transactions = Vec::new();
        let prefix_string = format!("block_tx:{}:", hex::encode(block_hash));
        for item in self.tx_by_block_tree.scan_prefix(prefix_string.as_bytes()) {
            let (_key, tx_hash_bytes) = item?;
            let tx_hash_array: [u8; 32] = tx_hash_bytes
                .as_ref()
                .try_into()
                .map_err(|_| CryptoError::HashConversionError)?;
            if let Some(tx) = self.get_transaction(&tx_hash_array)? {
                transactions.push(tx);
            }
        }
        // Transactions might not be in exact order if we don't sort after retrieval,
        // but for now, simple retrieval by block is the goal.
        Ok(transactions)
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
        for item in self
            .address_to_tx_tree
            .scan_prefix(prefix_string.as_bytes())
            .rev()
        {
            let (_key, tx_hash_bytes) = item?;
            let tx_hash_array: [u8; 32] = tx_hash_bytes
                .as_ref()
                .try_into()
                .map_err(|_| CryptoError::HashConversionError)?;
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
        let from_height_key = format!("height:{}", from_height);
        let _to_height_key = format!("height:{}", to_height);

        let mut iter = self
            .blocks_tree
            .scan_prefix(from_height_key.as_bytes())
            .rev();
        while let Some(Ok((_key, encoded))) = iter.next() {
            let block: Block = bincode::deserialize(&encoded)?;
            if block.index > to_height {
                break;
            }
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
        Ok(transactions)
    }

    fn get_block_hashes_by_height_range(
        &self,
        from_height: u64,
        to_height: u64,
    ) -> Result<Vec<[u8; 32]>, StorageError> {
        let mut block_hashes = Vec::new();
        let from_height_key = format!("height:{}", from_height);
        let _to_height_key = format!("height:{}", to_height);

        let mut iter = self
            .blocks_tree
            .scan_prefix(from_height_key.as_bytes())
            .rev();
        while let Some(Ok((_key, encoded))) = iter.next() {
            let block: Block = bincode::deserialize(&encoded)?;
            if block.index > to_height {
                break;
            }
            block_hashes.push(block.hash);
        }
        Ok(block_hashes)
    }

    fn get_account_transaction_count(&self, address: &PublicKey) -> Result<u64, StorageError> {
        let key = hex::encode(address.to_bytes());
        let count_bytes = self
            .tx_count_tree
            .get(key.as_bytes())?
            .ok_or(StorageError::NotFound)?;
        Ok(String::from_utf8_lossy(&count_bytes)
            .parse::<u64>()
            .unwrap_or(0))
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
        for item in self
            .contract_to_tx_tree
            .scan_prefix(prefix_string.as_bytes())
            .rev()
        {
            let (_key, tx_hash_bytes) = item?;
            let tx_hash_array: [u8; 32] = tx_hash_bytes
                .as_ref()
                .try_into()
                .map_err(|_| CryptoError::HashConversionError)?;
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
        self.accounts_tree.insert(address.to_bytes(), encoded)?;
        self.accounts_tree.flush()?;
        Ok(())
    }

    fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, StorageError> {
        let encoded = self.accounts_tree.get(address.to_bytes())?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn delete_account(&self, address: &PublicKey) -> Result<(), StorageError> {
        self.accounts_tree.remove(address.to_bytes())?;
        Ok(())
    }

    fn get_all_accounts(&self) -> Result<Vec<(PublicKey, Account)>, StorageError> {
        let mut accounts = Vec::new();
        for item in self.accounts_tree.iter() {
            let (key, value) = item?;
            let pk = PublicKey::from_bytes(
                &key.as_ref()
                    .try_into()
                    .map_err(|_| StorageError::Crypto(CryptoError::InvalidPublicKey))?,
            )?;
            let account: Account = bincode::deserialize(&value)?;
            accounts.push((pk, account));
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
        self.contract_code_tree.insert(contract_id.id, wasm_bytes)?;
        Ok(())
    }

    fn get_contract_code(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError> {
        let encoded = self.contract_code_tree.get(contract_id.id)?;
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
        // Flush all trees to ensure durability and trigger sled cleanup
        self.blocks_tree.flush()?;
        self.transactions_tree.flush()?;
        self.mempool_tree.flush()?;
        self.accounts_tree.flush()?;
        self.chain_state_tree.flush()?;
        self.contract_code_tree.flush()?;
        self.contract_storage_tree.flush()?;
        self.height_to_block_tree.flush()?;
        self.tx_by_block_tree.flush()?;
        self.address_to_tx_tree.flush()?;
        self.contract_to_tx_tree.flush()?;
        self.tx_count_tree.flush()?;
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
        export_tree(
            &self.transactions_tree,
            &backup_db.open_tree("transactions")?,
        )?;
        export_tree(&self.mempool_tree, &backup_db.open_tree("mempool")?)?;
        export_tree(&self.accounts_tree, &backup_db.open_tree("accounts")?)?;
        export_tree(
            &self.chain_state_tree,
            &backup_db.open_tree("chain_state")?,
        )?;
        export_tree(
            &self.contract_code_tree,
            &backup_db.open_tree("contract_code")?,
        )?;
        export_tree(
            &self.contract_storage_tree,
            &backup_db.open_tree("contract_storage")?,
        )?;
        export_tree(&self.height_to_block_tree, &backup_db.open_tree("height_to_block")?)?;
        export_tree(
            &self.tx_by_block_tree,
            &backup_db.open_tree("tx_by_block")?,
        )?;
        export_tree(
            &self.address_to_tx_tree,
            &backup_db.open_tree("address_to_tx")?,
        )?;
        export_tree(
            &self.contract_to_tx_tree,
            &backup_db.open_tree("contract_to_tx")?,
        )?;
        export_tree(&self.tx_count_tree, &backup_db.open_tree("tx_count")?)?;

        backup_db.flush()?;
        log::info!("Storage backup completed to {:?}", path);
        Ok(())
    }

    fn restore_from(&self, path: &std::path::Path) -> Result<(), StorageError> {
        let backup_db = sled::open(path).map_err(StorageError::Sled)?;

        let restore_tree =
            |src_tree: &sled::Tree, dst: &sled::Tree| -> Result<(), StorageError> {
                dst.clear()?;
                for item in src_tree.iter() {
                    let (key, value) = item?;
                    dst.insert(key, value)?;
                }
                dst.flush()?;
                Ok(())
            };

        restore_tree(
            &backup_db.open_tree("blocks")?,
            &self.blocks_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("transactions")?,
            &self.transactions_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("mempool")?,
            &self.mempool_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("accounts")?,
            &self.accounts_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("chain_state")?,
            &self.chain_state_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("contract_code")?,
            &self.contract_code_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("contract_storage")?,
            &self.contract_storage_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("height_to_block")?,
            &self.height_to_block_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("tx_by_block")?,
            &self.tx_by_block_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("address_to_tx")?,
            &self.address_to_tx_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("contract_to_tx")?,
            &self.contract_to_tx_tree,
        )?;
        restore_tree(
            &backup_db.open_tree("tx_count")?,
            &self.tx_count_tree,
        )?;

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

    fn clone_storage(&self) -> Box<dyn Storage> {
        Box::new(self.clone())
    }

    fn clear_mempool(&self) -> Result<(), StorageError> {
        for item in self.mempool_tree.scan_prefix("pending:") {
            let (_key, _value) = item?;
            self.mempool_tree.remove(_key)?;
        }
        Ok(())
    }
}
