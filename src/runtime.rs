use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use rand::Rng;
use rand::rngs::OsRng;
use ed25519_dalek::SigningKey;

use crate::types::{Block, ChainState, Transaction, Account, CryptoError, PublicKey, ContractId};
use crate::storage::{Storage, StorageError};
use crate::ledger::{Ledger, LedgerError};
use crate::consensus::{ConsensusEngine, ConsensusError};
use crate::contracts::BaaLSContractEngine;
use crate::sync::SyncLayer;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Ledger error: {0}")]
    LedgerError(#[from] LedgerError),
    #[error("Consensus error: {0}")]
    ConsensusError(#[from] ConsensusError),
    #[error("Crypto error: {0}")]
    CryptoError(#[from] CryptoError),
    #[error("Failed to initialize chain")]
    ChainInitializationError,
    #[error("Failed to create new keypair")]
    KeypairGenerationError,
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),
    #[error("Runtime already running")]
    AlreadyRunning,
    #[error("Runtime not running")]
    NotRunning,
}

pub struct Runtime<S: Storage, C: ConsensusEngine, Y: SyncLayer> {
    storage: Arc<S>,
    ledger: Arc<Ledger<S, BaaLSContractEngine<S>>>,
    consensus: Arc<C>,
    mempool: Arc<Mutex<Vec<Transaction>>>,
    chain_state: Arc<Mutex<ChainState>>,
    _is_running: Arc<Mutex<bool>>,
    sync_layer: Arc<Y>,
    contract_engine_arc: Arc<BaaLSContractEngine<S>>,
}

impl<S: Storage + 'static, C: ConsensusEngine + 'static, Y: SyncLayer + 'static> Runtime<S, C, Y> {
    pub fn new(storage: S, consensus: C, contract_engine: BaaLSContractEngine<S>, sync_layer: Y) -> Result<Self, RuntimeError> {
        let storage_arc = Arc::new(storage);
        let contract_engine_arc = Arc::new(contract_engine);
        let ledger = Arc::new(Ledger::new(Arc::clone(&storage_arc), Arc::clone(&contract_engine_arc)));

        // Initialize chain if not already initialized
        ledger.initialize_chain()?;

        let initial_chain_state = storage_arc.get_chain_state()?.ok_or(RuntimeError::ChainInitializationError)?;

        Ok(Runtime {
            storage: storage_arc,
            ledger,
            consensus: Arc::new(consensus),
            mempool: Arc::new(Mutex::new(Vec::new())),
            chain_state: Arc::new(Mutex::new(initial_chain_state)),
            _is_running: Arc::new(Mutex::new(false)),
            sync_layer: Arc::new(sync_layer),
            contract_engine_arc,
        })
    }

    pub fn generate_keypair() -> Result<SigningKey, RuntimeError> {
        let mut csprng = OsRng;
        // Use random bytes to create a signing key
        let mut secret_key_bytes = [0u8; 32];
        csprng.fill(&mut secret_key_bytes);
        Ok(SigningKey::from_bytes(&secret_key_bytes))
    }

    pub fn start(&self) -> Result<(), RuntimeError> {
        println!("BaaLS Runtime started");
        
        // For now, just start the sync layer without async spawning
        // TODO: Implement proper async runtime management
        Ok(())
    }

    pub fn stop(&self) -> Result<(), RuntimeError> {
        println!("BaaLS Runtime stopped");
        Ok(())
    }

    pub fn submit_transaction(&self, transaction: Transaction) -> Result<(), RuntimeError> {
        // Basic validation for MVP
        if !transaction.verify_signature()? {
            return Err(RuntimeError::InvalidTransaction("Invalid transaction signature".to_string()));
        }

        // Check sender account nonce from current chain state
        let _current_chain_state = self.chain_state.lock().unwrap();
        let sender_pk = transaction.sender;
        let sender_account = self.storage.get_account(&sender_pk)?.unwrap_or_else(|| {
            // If account doesn't exist, allow it for now, Ledger will create it for transfers.
            // For production, stricter rules might apply, e.g., requiring initial balance.
            Account::Wallet { balance: 0, nonce: 0 }
        });

        if transaction.nonce <= sender_account.nonce() {
            return Err(RuntimeError::InvalidTransaction(format!("Invalid nonce: expected greater than {}, got {}", sender_account.nonce(), transaction.nonce)));
        }
        // For MVP, we're not handling out-of-order nonces in mempool explicitly.
        // This will be handled by ledger during block application.

        let hash = transaction.hash;
        self.mempool.lock().unwrap().push(transaction);
        println!("Transaction submitted: {}", crate::types::format_hex(&hash));
        Ok(())
    }

    pub async fn produce_block(&self) -> Result<Block, RuntimeError> {
        let mempool = self.mempool.lock().unwrap();
        if mempool.is_empty() {
            return Err(ConsensusError::NoPendingTransactions.into());
        }

        let current_chain_state = self.chain_state.lock().unwrap();
        let prev_block = self.storage.get_block(&current_chain_state.latest_block_hash)?.ok_or(StorageError::NotFound)?;

        let new_block = self.consensus.generate_block(&mempool, &prev_block, &current_chain_state)?;
        
        // Release mempool lock before acquiring chain_state lock to avoid deadlock if called from external thread
        drop(mempool);

        let mut current_chain_state_mut = self.chain_state.lock().unwrap();

        // Validate and apply block to ledger
        self.ledger.validate_block(&new_block, &current_chain_state_mut)?;
        // Pass contract_engine to apply_block
        self.ledger.apply_block(new_block.clone(), &mut current_chain_state_mut)?;

        println!("Block produced and applied: {}", crate::types::format_hex(&new_block.hash));

        // Optionally broadcast the new block
        let sync_layer_clone = Arc::clone(&self.sync_layer);
        let new_block_clone = new_block.clone();
        tokio::spawn(async move {
            let peers = sync_layer_clone.discover_peers().await.unwrap_or_else(|e| {
                eprintln!("Error discovering peers: {}", e);
                Vec::new()
            });
            if let Err(e) = sync_layer_clone.broadcast_block(&new_block_clone, &peers).await {
                eprintln!("Error broadcasting block: {}", e);
            }
        });

        // Clear included transactions from mempool (this would be more sophisticated in real impl)
        // For MVP, we clear all for simplicity after block generation.
        self.mempool.lock().unwrap().clear();

        Ok(new_block)
    }

    pub fn get_chain_state(&self) -> Result<ChainState, RuntimeError> {
        Ok(self.chain_state.lock().unwrap().clone())
    }

    pub fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, RuntimeError> {
        Ok(self.storage.get_block(hash)?)
    }

    pub fn get_transaction(&self, tx_hash: &[u8; 32]) -> Result<Option<Transaction>, RuntimeError> {
        Ok(self.storage.get_transaction(tx_hash)?)
    }

    pub fn contract_engine(&self) -> &BaaLSContractEngine<S> {
        self.contract_engine_arc.as_ref()
    }

    // Utility function to generate a new signing key
    pub fn generate_signing_key() -> Result<SigningKey, RuntimeError> {
        let mut csprng = OsRng;
        let mut secret_key_bytes = [0u8; 32];
        csprng.fill(&mut secret_key_bytes);
        Ok(SigningKey::from_bytes(&secret_key_bytes))
    }

    pub fn get_current_timestamp(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    pub fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, RuntimeError> {
        self.storage.get_block_by_height(height).map_err(RuntimeError::StorageError)
    }

    pub fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, RuntimeError> {
        self.storage.get_account(address).map_err(RuntimeError::StorageError)
    }

    pub fn contract_storage_read(&self, contract_id: &ContractId, key: &[u8]) -> Result<Option<Vec<u8>>, RuntimeError> {
        self.storage.contract_storage_read(contract_id, key).map_err(RuntimeError::StorageError)
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }
} 