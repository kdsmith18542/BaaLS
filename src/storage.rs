//! Persistent storage layer for BaaLS blockchain.
//!
//! This module provides an abstraction over the underlying storage engine (sled)
//! for persisting blocks, transactions, accounts, and contract state.

use bincode;
use hex;
use sled::{Db, Tree};
use std::path::Path;
use thiserror::Error;

use crate::types::PublicKey;
use crate::types::{Account, Block, ChainState, ContractId, CryptoError, Transaction};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sled::Error),
    #[error("Data not found")]
    NotFound,
    #[error("Serialization error: {0}")]
    SerializationError(#[from] bincode::Error),
    #[error("Crypto error: {0}")]
    CryptoError(#[from] CryptoError),
}

/// Storage abstraction for blockchain persistence.
///
/// This trait defines the interface for storing and retrieving blockchain data.
/// Implementations must be thread-safe (Send + Sync).
pub trait Storage: Send + Sync {
    /// Store a block in the database.
    fn put_block(&self, block: &Block) -> Result<(), StorageError>;

    /// Retrieve a block by its hash.
    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StorageError>;

    /// Get the latest block in the chain.
    fn get_latest_block(&self) -> Result<Option<Block>, StorageError>;

    /// Get the current chain height (latest block index).
    fn get_chain_height(&self) -> Result<u64, StorageError>;

    /// Retrieve a block by its height (index).
    fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError>;

    // Transaction Management

    /// Store a transaction in the database.
    fn put_transaction(&self, tx: &Transaction) -> Result<(), StorageError>;

    /// Retrieve a transaction by its hash.
    fn get_transaction(&self, tx_hash: &[u8; 32]) -> Result<Option<Transaction>, StorageError>;

    /// Get all pending transactions from the mempool.
    fn get_pending_transactions(&self) -> Result<Vec<Transaction>, StorageError>;

    /// Remove a pending transaction from the mempool.
    fn remove_pending_transaction(&self, tx_hash: &[u8; 32]) -> Result<(), StorageError>;

    // New: Transaction indexing for fast lookup
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

    // Account State Management (used by Ledger)

    /// Store an account's state.
    fn put_account(&self, address: &PublicKey, account: &Account) -> Result<(), StorageError>;

    /// Retrieve an account's state.
    fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, StorageError>;

    /// Delete an account from storage.
    fn delete_account(&self, address: &PublicKey) -> Result<(), StorageError>;

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
}

#[derive(Default)]
pub struct StorageBatch {
    pub ops: Vec<StorageOperation>,
}

pub enum StorageOperation {
    Put(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
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
}

impl SledStorage {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db = sled::open(path)?;
        Ok(Self {
            blocks_tree: db.open_tree("blocks")?,
            transactions_tree: db.open_tree("transactions")?,
            mempool_tree: db.open_tree("mempool")?,
            accounts_tree: db.open_tree("accounts")?,
            contract_code_tree: db.open_tree("contract_code")?,
            contract_storage_tree: db.open_tree("contract_storage")?,
            chain_state_tree: db.open_tree("chain_state")?,
            tx_by_block_tree: db.open_tree("tx_by_block")?,
            db,
        })
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

    fn put_account(&self, address: &PublicKey, account: &Account) -> Result<(), StorageError> {
        let encoded = bincode::serialize(account)?;
        self.accounts_tree.insert(address.to_bytes(), encoded)?;
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

    fn put_chain_state(&self, state: &ChainState) -> Result<(), StorageError> {
        let encoded = bincode::serialize(state)?;
        self.chain_state_tree.insert("global:current", encoded)?;
        Ok(())
    }

    fn get_chain_state(&self) -> Result<Option<ChainState>, StorageError> {
        let encoded = self.chain_state_tree.get("global:current")?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
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
        let mut tree_batch = sled::Batch::default();
        for op in batch.ops {
            match op {
                StorageOperation::Put(key, value) => {
                    tree_batch.insert(key, value);
                }
                StorageOperation::Delete(key) => {
                    tree_batch.remove(key);
                }
            }
        }
        self.db.apply_batch(tree_batch)?;
        Ok(())
    }
}
