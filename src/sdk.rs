use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

use ed25519_dalek::SigningKey;
use sha2::Digest;

use crate::consensus::PoAConsensus;
use crate::contracts::{BaaLSContractEngine, ContractEngine};
use crate::runtime::{MempoolStats, Runtime};
use crate::storage::{SledStorage, Storage};
use crate::sync::NoopSync;
use crate::types::{Account, Block, ChainState, ContractId, PublicKey, Transaction};

#[derive(Debug, Error)]
pub enum SdkError {
    #[error("Runtime error: {0}")]
    RuntimeError(#[from] crate::runtime::RuntimeError),
    #[error("Storage error: {0}")]
    StorageError(#[from] crate::storage::StorageError),
    #[error("Contract error: {0}")]
    ContractError(#[from] crate::contracts::ContractError),
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Node not running")]
    NodeNotRunning,
}

/// High-level SDK for BaaLS blockchain
pub struct BaaLSSdk {
    runtime: Arc<Runtime<SledStorage, PoAConsensus, NoopSync>>,
    data_dir: PathBuf,
}

impl BaaLSSdk {
    /// Create a new BaaLS SDK instance
    pub fn new(data_dir: PathBuf) -> Result<Self, SdkError> {
        let storage = SledStorage::new(&data_dir)?;
        let mut secret = [0u8; 32];
        use rand::RngCore;
        rand::rng().fill_bytes(&mut secret);
        let signing_key = SigningKey::from_bytes(&secret);
        let test_key = PublicKey::from(signing_key.verifying_key());
        let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
        let contract_engine = BaaLSContractEngine::new(storage.clone())?;
        let sync_layer = NoopSync;

        let runtime = Runtime::new(storage, consensus, contract_engine, sync_layer)?;

        Ok(Self { runtime: Arc::new(runtime), data_dir })
    }

    /// Create a new BaaLS SDK instance with custom mempool size limit
    pub fn with_mempool_limit(
        data_dir: PathBuf,
        mempool_size_limit: usize,
    ) -> Result<Self, SdkError> {
        let storage = SledStorage::new(&data_dir)?;
        let mut secret = [0u8; 32];
        use rand::RngCore;
        rand::rng().fill_bytes(&mut secret);
        let signing_key = SigningKey::from_bytes(&secret);
        let test_key = PublicKey::from(signing_key.verifying_key());
        let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
        let contract_engine = BaaLSContractEngine::new(storage.clone())?;
        let sync_layer = NoopSync;

        let runtime = Runtime::with_mempool_limit(
            storage,
            consensus,
            contract_engine,
            sync_layer,
            mempool_size_limit,
        )?;

        Ok(Self { runtime: Arc::new(runtime), data_dir })
    }

    /// Start the BaaLS node
    pub fn start(&self) -> Result<(), SdkError> {
        self.runtime.start()?;
        Ok(())
    }

    /// Stop the BaaLS node
    pub fn stop(&self) -> Result<(), SdkError> {
        self.runtime.stop()?;
        Ok(())
    }

    /// Submit a transaction to the mempool
    pub fn submit_transaction(&self, transaction: Transaction) -> Result<(), SdkError> {
        self.runtime.submit_transaction(transaction)?;
        Ok(())
    }

    /// Generate a new block
    pub async fn produce_block(&self) -> Result<Block, SdkError> {
        let block = self.runtime.produce_block().await?;
        Ok(block)
    }

    /// Get the current chain state
    pub fn get_chain_state(&self) -> Result<ChainState, SdkError> {
        let state = self.runtime.get_node_status()?;
        Ok(state)
    }

    /// Get a block by its hash
    pub fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, SdkError> {
        let block = self.runtime.get_block_by_hash(hash)?;
        Ok(block)
    }

    /// Get a block by its height
    pub fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, SdkError> {
        let block = self.runtime.get_block_by_height(height)?;
        Ok(block)
    }

    /// Get a transaction by its hash
    pub fn get_transaction(&self, tx_hash: &[u8; 32]) -> Result<Option<Transaction>, SdkError> {
        let tx = self.runtime.get_transaction(tx_hash)?;
        Ok(tx)
    }

    /// Get an account by its address
    pub fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, SdkError> {
        let account = self.runtime.get_account(address)?;
        Ok(account)
    }

    /// Create a new account with initial balance
    pub fn create_account(&self, address: &PublicKey, account: Account) -> Result<(), SdkError> {
        self.runtime.create_account(address, account)?;
        Ok(())
    }

    /// Get mempool statistics
    pub fn get_mempool_stats(&self) -> Result<MempoolStats, SdkError> {
        let stats = self.runtime.get_mempool_stats()?;
        Ok(stats)
    }

    /// Get all pending transactions in the mempool
    pub fn get_mempool(&self) -> Result<Vec<Transaction>, SdkError> {
        let mempool = self.runtime.get_mempool()?;
        Ok(mempool)
    }

    /// Read from contract storage
    pub fn contract_storage_read(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, SdkError> {
        let value = self.runtime.contract_storage_read(contract_id, key)?;
        Ok(value)
    }

    /// Deploy a smart contract
    pub fn deploy_contract(
        &self,
        deployer: &PublicKey,
        wasm_bytes: &[u8],
        init_payload: Option<&[u8]>,
        gas_limit: u64,
    ) -> Result<ContractId, SdkError> {
        let deployer_account = self.runtime.storage().get_account(deployer)?;
        let deployer_nonce = deployer_account.as_ref().map(|a| a.nonce()).unwrap_or(0);
        let deploy_result = self.runtime.contract_engine().deploy_contract(
            deployer,
            deployer_nonce,
            wasm_bytes,
            init_payload,
            self.runtime.storage(),
            gas_limit,
        )?;
        // Store contract code (immutable, idempotent)
        self.runtime.storage().put_contract_code(&deploy_result.contract_id, &deploy_result.wasm_bytes)?;
        self.runtime.storage().put_contract_deployer(&deploy_result.contract_id, &deploy_result.deployer)?;
        // Apply init side effects
        for (key, val) in deploy_result.side_effects.storage_updates.writes {
            self.runtime.storage().contract_storage_write(&deploy_result.contract_id, &key, &val)?;
        }
        for key in deploy_result.side_effects.storage_updates.deletes {
            self.runtime.storage().contract_storage_remove(&deploy_result.contract_id, &key)?;
        }
        Ok(deploy_result.contract_id)
    }

    /// Call a smart contract
    pub fn call_contract(
        &self,
        caller: &PublicKey,
        contract_id: &ContractId,
        method_name: &str,
        args: &[Vec<u8>],
        value: Option<u64>,
    ) -> Result<Vec<u8>, SdkError> {
        let block_index =
            self.runtime.get_chain_state().map(|cs| cs.latest_block_index).unwrap_or(0);
        let block_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let result = self.runtime.contract_engine().call_contract(
            caller,
            contract_id,
            method_name,
            args,
            value,
            self.runtime.storage(),
            block_index,
            block_timestamp,
            1_000_000,
        )?;
        Ok(result.output)
    }

    /// Query a smart contract
    pub fn query_contract(
        &self,
        contract_id: &ContractId,
        method_name: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, SdkError> {
        let block_index =
            self.runtime.get_chain_state().map(|cs| cs.latest_block_index).unwrap_or(0);
        let block_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let result = self.runtime.contract_engine().query_contract(
            contract_id,
            method_name,
            payload,
            self.runtime.storage(),
            block_index,
            block_timestamp,
        )?;
        Ok(result)
    }

    /// Verify a smart contract
    pub fn verify_contract(&self, wasm_bytes: &[u8]) -> Result<(), SdkError> {
        let mut hasher = sha2::Sha256::new();
        hasher.update(wasm_bytes);
        let hash: [u8; 32] = hasher.finalize().into();
        let contract_id = ContractId::from_bytes(&hash);
        self.runtime.contract_engine().verify_contract(wasm_bytes, &contract_id)?;
        Ok(())
    }

    /// Get transaction history for an account
    pub fn get_transaction_history(
        &self,
        address: &PublicKey,
        limit: usize,
    ) -> Result<Vec<Transaction>, SdkError> {
        let history = self.runtime.get_transaction_history(address, limit)?;
        Ok(history)
    }

    /// Get the data directory path
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }
}

/// Builder pattern for BaaLSSdk configuration
pub struct BaaLSSdkBuilder {
    data_dir: Option<PathBuf>,
    mempool_size_limit: Option<usize>,
}

impl BaaLSSdkBuilder {
    pub fn new() -> Self {
        Self { data_dir: None, mempool_size_limit: None }
    }

    pub fn data_dir(mut self, data_dir: PathBuf) -> Self {
        self.data_dir = Some(data_dir);
        self
    }

    pub fn mempool_size_limit(mut self, limit: usize) -> Self {
        self.mempool_size_limit = Some(limit);
        self
    }

    pub fn build(self) -> Result<BaaLSSdk, SdkError> {
        let data_dir = self.data_dir.unwrap_or_else(|| PathBuf::from("./data"));

        match self.mempool_size_limit {
            Some(limit) => BaaLSSdk::with_mempool_limit(data_dir, limit),
            None => BaaLSSdk::new(data_dir),
        }
    }
}

impl Default for BaaLSSdkBuilder {
    fn default() -> Self {
        Self::new()
    }
}
