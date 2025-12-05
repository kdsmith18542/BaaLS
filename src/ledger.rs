use thiserror::Error;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::types::{Block, ChainState, Account, CryptoError, TransactionPayload, PublicKey};
use crate::storage::{Storage, StorageError, StorageBatch, StorageOperation};
use crate::contracts::ContractEngine;

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Crypto error: {0}")]
    CryptoError(#[from] CryptoError),
    #[error("Block validation failed: {0}")]
    BlockValidation(String),
    #[error("State transition failed: {0}")]
    StateTransition(String),
    #[error("Account not found for address: {0}")]
    AccountNotFound(String),
    #[error("Insufficient balance for transaction from {0}")]
    InsufficientBalance(String),
    #[error("Invalid nonce for account {0}: expected {1}, got {2}")]
    InvalidNonce(String, u64, u64),
    #[error("Contract not found: {0}")]
    ContractNotFound(String),
    #[error("WASM validation failed: {0}")]
    WasmValidationFailed(String),
    #[error("Invalid transaction payload")]
    InvalidTransactionPayload,
    #[error("Serialization error: {0}")]
    SerializationError(#[from] Box<bincode::ErrorKind>),
    #[error("Contract error: {0}")]
    ContractError(#[from] crate::contracts::ContractError),
    #[error("Not found")]
    NotFound,
}

pub struct Ledger<S: Storage, C: ContractEngine> {
    storage: Arc<S>,
    contract_engine: Arc<C>,
}

impl<S: Storage, C: ContractEngine> Ledger<S, C> {
    pub fn new(storage: Arc<S>, contract_engine: Arc<C>) -> Self {
        Ledger { storage, contract_engine }
    }

    pub fn initialize_chain(&self) -> Result<(), LedgerError> {
        // Check if chain state already exists
        if self.storage.get_chain_state()?.is_some() {
            println!("Chain already initialized.");
            return Ok(());
        }

        // Create a genesis block
        let genesis_block = Block {
            index: 0,
            timestamp: 0,
            prev_hash: [0; 32], // Genesis block has no previous hash
            hash: [0; 32], // Will be calculated after creation
            nonce: 0,
            transactions: Vec::new(),
            metadata: None,
        };

        let calculated_genesis_hash = genesis_block.calculate_hash()?;
        let mut genesis_block = genesis_block;
        genesis_block.hash = calculated_genesis_hash;

        let initial_chain_state = ChainState {
            latest_block_hash: genesis_block.hash,
            latest_block_index: 0,
            accounts_root_hash: [0; 32], // Placeholder, will be updated by Merkle tree impl
            total_supply: 0, // No native token for now
        };

        let mut batch = StorageBatch::default();
        batch.ops.push(StorageOperation::Put(
            bincode::serialize(&genesis_block.hash)?,
            bincode::serialize(&genesis_block)?,
        ));
        batch.ops.push(StorageOperation::Put(
            bincode::serialize("global:current")?,
            bincode::serialize(&initial_chain_state)?,
        ));

        self.storage.apply_batch(batch)?;
        println!("Chain initialized with genesis block: {}", crate::types::format_hex(&genesis_block.hash));
        Ok(())
    }

    pub fn validate_block(&self, block: &Block, current_chain_state: &ChainState) -> Result<(), LedgerError> {
        // Basic Block Header Validation
        if block.index != current_chain_state.latest_block_index + 1 {
            return Err(LedgerError::BlockValidation(format!(
                "Invalid block index: expected {}, got {}",
                current_chain_state.latest_block_index + 1,
                block.index
            )));
        }
        if block.prev_hash != current_chain_state.latest_block_hash {
            return Err(LedgerError::BlockValidation(format!(
                "Invalid previous hash: expected {:x?}, got {:x?}",
                current_chain_state.latest_block_hash,
                block.prev_hash
            )));
        }

        let calculated_hash = block.calculate_hash()?;
        if calculated_hash != block.hash {
            return Err(LedgerError::BlockValidation(format!(
                "Invalid block hash: expected {:x?}, got {:x?}",
                calculated_hash,
                block.hash
            )));
        }

        // Timestamp check (simplified for MVP, typically more robust logic needed)
        if block.index > 0 && block.timestamp <= self.storage.get_block(&block.prev_hash)?.ok_or(LedgerError::NotFound)?.timestamp {
            return Err(LedgerError::BlockValidation(
                "Block timestamp is not greater than previous block's timestamp".to_string()
            ));
        }

        // Transaction Validation (within the block) - only basic checks for MVP
        for tx in &block.transactions {
            if !tx.verify_signature()? {
                return Err(LedgerError::BlockValidation(
                    format!("Invalid signature for transaction: {:x?}", tx.hash)
                ));
            }
            // Further transaction validation (nonce, balance) will happen during state transition
        }

        Ok(())
    }

    pub fn apply_block(
        &self,
        block: Block,
        current_chain_state: &mut ChainState,
    ) -> Result<(), LedgerError> {
        let mut batch = StorageBatch::default();
        let mut accounts_to_update: BTreeMap<PublicKey, Account> = BTreeMap::new();

        for tx in &block.transactions {
            let sender_pk = tx.sender;
            let mut sender_account = self.storage.get_account(&sender_pk)?.ok_or_else(|| {
                LedgerError::AccountNotFound(format!("Sender account not found: {:?}", sender_pk))
            })?;

            // Nonce Check
            if sender_account.nonce() + 1 != tx.nonce {
                return Err(LedgerError::InvalidNonce(
                    format!("{:?}", sender_pk),
                    sender_account.nonce() + 1,
                    tx.nonce,
                ));
            }
            sender_account.set_nonce(sender_account.nonce() + 1);
            accounts_to_update.insert(sender_pk, sender_account.clone());

            match &tx.payload {
                TransactionPayload::Transfer { amount } => {
                    if let Account::Wallet { balance, .. } = accounts_to_update.get_mut(&tx.sender).unwrap() {
                        if *balance < *amount {
                            return Err(LedgerError::InsufficientBalance(format!("{:?}", tx.sender)));
                        }
                        *balance -= *amount;
                    } else {
                        return Err(LedgerError::StateTransition("Sender is not a wallet account".to_string()));
                    }

                    if let Some(mut recipient_account) = match tx.recipient {
                        crate::types::Address::Wallet(pk) => self.storage.get_account(&pk)?,
                        crate::types::Address::Contract(_) => return Err(LedgerError::StateTransition("Cannot transfer native token to a contract directly".to_string())),
                    } {
                        if let Account::Wallet { balance, .. } = &mut recipient_account {
                            *balance += amount;
                            accounts_to_update.insert(match tx.recipient { crate::types::Address::Wallet(pk) => pk, _ => unreachable!()}, recipient_account);
                        } else {
                            return Err(LedgerError::StateTransition("Recipient is not a wallet account".to_string()));
                        }
                    } else { // Create new account if recipient doesn't exist
                        if let crate::types::Address::Wallet(pk) = tx.recipient {
                            accounts_to_update.insert(pk, Account::Wallet { balance: *amount, nonce: 0 });
                        } else {
                             // Should be unreachable due to previous check
                        }
                    }
                },
                TransactionPayload::ContractDeploy { wasm_bytes } => {
                    // Full WASM validation/execution in ContractEngine module.
                    let contract_id = self.contract_engine.deploy_contract(
                        &tx.sender,
                        &wasm_bytes,
                        None, // No init_payload in new variant
                        self.storage.as_ref(),
                        tx.gas_limit,
                    )?;
                    // Update sender account to reflect new contract (if it's a contract account)
                    accounts_to_update.insert(tx.sender, Account::Contract {
                        code_hash: contract_id.id, // Use actual contract ID hash
                        storage_root_hash: [0; 32], // Placeholder, will be updated by Merkle tree impl
                        nonce: sender_account.nonce(),
                    });
                },
                TransactionPayload::ContractCall { method, args } => {
                    // Extract contract_id from recipient address
                    let contract_id = match &tx.recipient {
                        crate::types::Address::Contract(cid) => cid,
                        _ => return Err(LedgerError::InvalidTransactionPayload),
                    };
                    let _execution_result = self.contract_engine.call_contract(
                        &tx.sender,
                        contract_id,
                        method,
                        args,
                        self.storage.as_ref(),
                    );
                    // TODO: Handle execution result
                },
                TransactionPayload::Data { data: _ } => {
                    // For MVP, just allow storing data. No specific state changes yet.
                }
            }

            // Remove from mempool after successful processing
            batch.ops.push(StorageOperation::Delete(bincode::serialize(&tx.hash)?));
        }

        // Apply account updates (Merkle root calculation would go here in a full implementation)
        for (address, account) in accounts_to_update {
            batch.ops.push(StorageOperation::Put(
                bincode::serialize(&address)?,
                bincode::serialize(&account)?,
            ));
        }

        // Update chain state
        current_chain_state.latest_block_hash = block.hash;
        current_chain_state.latest_block_index = block.index;
        // Merkle root for accounts_root_hash would be calculated and updated here
        batch.ops.push(StorageOperation::Put(
            bincode::serialize("global:current")?,
            bincode::serialize(current_chain_state)?,
        ));

        // Index transactions by block hash as part of the batch
        for (i, tx) in block.transactions.iter().enumerate() {
            self.storage.index_transaction(&tx.hash, &block.hash, i as u32)?;
        }

        self.storage.apply_batch(batch)?;
        Ok(())
    }
} 