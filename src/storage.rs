ALL CODE MUST BE PRODUCTION GRADE AND ABSOLUTELY NO STUBS WITH OUT ASKING!!
use sled::{Db, Tree};
use bincode;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

use crate::types::{Block, Transaction, ChainState, Account, ContractId, CryptoError};

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

pub trait Storage: Send + Sync {
    fn put_block(&self, block: &Block) -> Result<(), StorageError>;
    fn get_block(&self, hash: &str) -> Result<Option<Block>, StorageError>;
    fn get_latest_block(&self) -> Result<Option<Block>, StorageError>;
    fn get_chain_height(&self) -> Result<u64, StorageError>;
    fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError>;

    fn put_transaction(&self, tx: &Transaction) -> Result<(), StorageError>;
    fn get_transaction(&self, tx_hash: &str) -> Result<Option<Transaction>, StorageError>;
    fn get_pending_transactions(&self) -> Result<Vec<Transaction>, StorageError>;
    fn remove_pending_transaction(&self, tx_hash: &str) -> Result<(), StorageError>;

    fn put_account(&self, address: &ed25519_dalek::PublicKey, account: &Account) -> Result<(), StorageError>;
    fn get_account(&self, address: &ed25519_dalek::PublicKey) -> Result<Option<Account>, StorageError>;
    fn delete_account(&self, address: &ed25519_dalek::PublicKey) -> Result<(), StorageError>;

    fn put_chain_state(&self, state: &ChainState) -> Result<(), StorageError>;
    fn get_chain_state(&self) -> Result<Option<ChainState>, StorageError>;

    fn put_contract_code(&self, contract_id: &ContractId, wasm_bytes: &[u8]) -> Result<(), StorageError>;
    fn get_contract_code(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError>;
    fn contract_storage_read(&self, contract_id: &ContractId, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError>;
    fn contract_storage_write(&self, contract_id: &ContractId, key: &[u8], value: &[u8]) -> Result<(), StorageError>;
    fn contract_storage_remove(&self, contract_id: &ContractId, key: &[u8]) -> Result<(), StorageError>;

    fn apply_batch(&self, batch: StorageBatch) -> Result<(), StorageError>;
}

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
            db,
        })
    }
}

impl Storage for SledStorage {
    fn put_block(&self, block: &Block) -> Result<(), StorageError> {
        let block_hash = block.hash.clone();
        let block_height = block.index;
        let encoded = bincode::serialize(block)?;

        self.blocks_tree.insert(format!("hash:{}", block_hash), encoded.clone())?;
        self.blocks_tree.insert(format!("height:{:0>20}", block_height), encoded)?;
        Ok(())
    }

    fn get_block(&self, hash: &str) -> Result<Option<Block>, StorageError> {
        let encoded = self.blocks_tree.get(format!("hash:{}", hash))?;
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
        let encoded = self.blocks_tree.get(format!("height:{:0>20}", height))?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn put_transaction(&self, tx: &Transaction) -> Result<(), StorageError> {
        let encoded = bincode::serialize(tx)?;
        self.transactions_tree.insert(format!("hash:{}", tx.hash), encoded)?;
        Ok(())
    }

    fn get_transaction(&self, tx_hash: &str) -> Result<Option<Transaction>, StorageError> {
        let encoded = self.transactions_tree.get(format!("hash:{}", tx_hash))?;
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

    fn remove_pending_transaction(&self, tx_hash: &str) -> Result<(), StorageError> {
        self.mempool_tree.remove(format!("pending:{}", tx_hash))?;
        Ok(())
    }

    fn put_account(&self, address: &ed25519_dalek::PublicKey, account: &Account) -> Result<(), StorageError> {
        let encoded = bincode::serialize(account)?;
        self.accounts_tree.insert(address.to_bytes(), encoded)?;
        Ok(())
    }

    fn get_account(&self, address: &ed25519_dalek::PublicKey) -> Result<Option<Account>, StorageError> {
        let encoded = self.accounts_tree.get(address.to_bytes())?;
        Ok(encoded.map(|e| bincode::deserialize(&e)).transpose()?)
    }

    fn delete_account(&self, address: &ed25519_dalek::PublicKey) -> Result<(), StorageError> {
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

    fn put_contract_code(&self, contract_id: &ContractId, wasm_bytes: &[u8]) -> Result<(), StorageError> {
        self.contract_code_tree.insert(format!("code:{}", contract_id.id), wasm_bytes)?;
        Ok(())
    }

    fn get_contract_code(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError> {
        let encoded = self.contract_code_tree.get(format!("code:{}", contract_id.id))?;
        Ok(encoded.map(|e| e.to_vec()))
    }

    fn contract_storage_read(&self, contract_id: &ContractId, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        let full_key = format!("state:{}:{}", contract_id.id, hex::encode(key));
        let encoded = self.contract_storage_tree.get(full_key)?;
        Ok(encoded.map(|e| e.to_vec()))
    }

    fn contract_storage_write(&self, contract_id: &ContractId, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        let full_key = format!("state:{}:{}", contract_id.id, hex::encode(key));
        self.contract_storage_tree.insert(full_key, value)?;
        Ok(())
    }

    fn contract_storage_remove(&self, contract_id: &ContractId, key: &[u8]) -> Result<(), StorageError> {
        let full_key = format!("state:{}:{}", contract_id.id, hex::encode(key));
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