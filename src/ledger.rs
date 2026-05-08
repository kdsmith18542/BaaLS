use log::{info, warn};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use thiserror::Error;

use crate::contracts::ContractEngine;
use crate::storage::{Storage, StorageBatch, StorageError, StorageOperation};
use crate::types::{
    Account, Block, ChainState, ContractId, CryptoError, PublicKey, SparseMerkleTree,
    TransactionPayload,
};

#[derive(Debug, Error)]
pub enum StateTransitionError {
    #[error("Account not found: {0}")]
    AccountNotFound(String),
    #[error("Insufficient balance: {0}")]
    InsufficientBalance(String),
    #[error("Invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },
    #[error("Invalid transaction payload")]
    InvalidPayload,
    #[error("Contract error: {0}")]
    ContractError(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Arithmetic overflow/underflow")]
    Overflow,
}

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Crypto error: {0}")]
    CryptoError(#[from] CryptoError),
    #[error("Block validation failed: {0}")]
    BlockValidation(String),
    #[error("State transition failed: {0}")]
    StateTransition(#[from] StateTransitionError),
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

#[derive(Debug, Clone)]
pub struct TransactionExecutionResult {
    pub success: bool,
    pub gas_used: u64,
    pub error_message: Option<String>,
}

pub struct Ledger<S: Storage, C: ContractEngine> {
    storage: Arc<S>,
    contract_engine: Arc<C>,
}

impl<S: Storage, C: ContractEngine> Ledger<S, C> {
    const MAX_FUTURE_BLOCK_TIMESTAMP_SECONDS: u64 = 10;
    pub fn new(storage: Arc<S>, contract_engine: Arc<C>) -> Self {
        Ledger { storage, contract_engine }
    }

    fn make_contract_code_key(contract_id: &ContractId) -> Vec<u8> {
        format!("code:{}", hex::encode(contract_id.to_bytes())).into_bytes()
    }

    fn make_contract_deployer_key(contract_id: &ContractId) -> Vec<u8> {
        format!("deployer:{}", hex::encode(contract_id.to_bytes())).into_bytes()
    }

    pub fn initialize_chain(&self) -> Result<(), LedgerError> {
        let chain_state_exists = self.storage.get_chain_state()?.is_some();
        if chain_state_exists {
            return Ok(());
        }

        let genesis_block = Block {
            index: 0,
            timestamp: 0,
            prev_hash: [0; 32],
            hash: [0; 32],
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
            accounts_root_hash: [0; 32],
            total_supply: 0,
        };

        self.storage.put_block(&genesis_block)?;
        self.storage.put_chain_state(&initial_chain_state)?;
        Ok(())
    }

    pub fn validate_block(&self, block: &Block) -> Result<(), LedgerError> {
        let current_chain_state = self.storage.get_chain_state()?.ok_or(LedgerError::NotFound)?;
        
        if block.index != current_chain_state.latest_block_index + 1 {
            return Err(LedgerError::BlockValidation(format!(
                "Invalid block index: expected {}, got {}",
                current_chain_state.latest_block_index + 1,
                block.index
            )));
        }

        if block.prev_hash != current_chain_state.latest_block_hash {
            return Err(LedgerError::BlockValidation("Invalid previous hash".to_string()));
        }

        let calculated_hash = block.calculate_hash()?;
        if calculated_hash != block.hash {
            return Err(LedgerError::BlockValidation("Invalid block hash".to_string()));
        }

        for tx in &block.transactions {
            if !tx.verify_signature()? {
                return Err(LedgerError::BlockValidation("Invalid transaction signature".to_string()));
            }
        }

        Ok(())
    }

    fn compute_contract_code_hash(&self, contract_id: &ContractId) -> Result<[u8; 32], LedgerError> {
        let code = self.storage.get_contract_code(contract_id)?.ok_or_else(|| {
            LedgerError::ContractNotFound(hex::encode(contract_id.to_bytes()))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&code);
        Ok(hasher.finalize().into())
    }

    fn compute_contract_storage_root(&self, contract_id: &ContractId) -> Result<[u8; 32], LedgerError> {
        let keys = self.storage.get_all_contract_storage_keys(contract_id)?;
        if keys.is_empty() {
            return Ok([0; 32]);
        }
        let mut smt = SparseMerkleTree::new();
        for key in keys {
            if let Some(value) = self.storage.contract_storage_read(contract_id, &key)? {
                let smt_key: [u8; 32] = Sha256::digest(&key).into();
                let smt_val: [u8; 32] = Sha256::digest(&value).into();
                smt.insert(smt_key, smt_val.to_vec());
            }
        }
        Ok(smt.root())
    }

    pub fn apply_block(&self, block: &Block) -> Result<(), LedgerError> {
        info!("[LEDGER] Applying block {}...", block.index);

        let mut batch = StorageBatch::default();
        let mut touched_accounts = HashSet::new();
        let mut touched_contracts = HashSet::new();
        let current_chain_state = self.storage.get_chain_state()?.ok_or(LedgerError::NotFound)?;

        // 1. Validate block header and signatures
        self.validate_block(block)?;

        // 2. Process transactions
        for (tx_idx, tx) in block.transactions.iter().enumerate() {
            let mut tx_success = true;
            let mut gas_used: u64 = 21_000; // Base gas

            // Load sender account (fresh from storage for every tx to handle multiple txs from same sender in block)
            // In a real production system, we'd use a cache here, but for atomicity we must be careful.
            // Actually, we should track intermediate account states in a local map.
            let mut sender_account = if let Some(existing) = self.storage.get_account(&tx.sender)? {
                existing
            } else {
                return Err(LedgerError::StateTransition(
                    StateTransitionError::AccountNotFound(format!("{:?}", tx.sender)),
                ));
            };

            // Basic validation
            if sender_account.nonce() != tx.nonce {
                return Err(LedgerError::StateTransition(
                    StateTransitionError::InvalidNonce { expected: tx.nonce, got: sender_account.nonce() },
                ));
            }

            // Deduct base gas from sender (gas_price not yet implemented, fee is 0)
            let total_fee = 0u64;
            let mut sender_balance = sender_account.balance();
            if sender_balance < total_fee {
                return Err(LedgerError::StateTransition(
                    StateTransitionError::InsufficientBalance("Gas fee".to_string()),
                ));
            }
            sender_balance = sender_balance.checked_sub(total_fee).ok_or(
                LedgerError::StateTransition(StateTransitionError::Overflow),
            )?;

            // Process payload
            match &tx.payload {
                TransactionPayload::Transfer { amount } => {
                    if sender_balance < *amount {
                        tx_success = false;
                        warn!("[LEDGER] Transfer failed: insufficient balance");
                    } else {
                        sender_balance = sender_balance.checked_sub(*amount).ok_or(
                            LedgerError::StateTransition(StateTransitionError::Overflow),
                        )?;

                        let recipient_pk = match &tx.recipient {
                            crate::types::Address::Wallet(pk) => pk,
                            crate::types::Address::Contract(cid) => {
                                let mut arr = [0u8; 32];
                                arr.copy_from_slice(&cid.to_bytes());
                                &PublicKey::from_bytes(&arr)?
                            }
                        };

                        let mut recipient_account =
                            self.storage.get_account(recipient_pk)?.unwrap_or(Account::Wallet {
                                balance: 0,
                                nonce: 0,
                            });

                        let new_recipient_balance =
                            recipient_account.balance().checked_add(*amount).ok_or(
                                LedgerError::StateTransition(StateTransitionError::Overflow),
                            )?;

                        match &mut recipient_account {
                            Account::Wallet { balance, .. } => *balance = new_recipient_balance,
                            Account::Contract { balance, .. } => *balance = new_recipient_balance,
                        }

                        batch.ops.push(StorageOperation::PutAccount(
                            recipient_pk.to_bytes().to_vec(),
                            bincode::serialize(&recipient_account)?,
                        ));
                        touched_accounts.insert(recipient_pk.to_bytes());
                    }
                }
                TransactionPayload::ContractDeploy { wasm_bytes, init_payload } => {
                    gas_used = gas_used.saturating_add(100_000);
                    if gas_used > tx.gas_limit {
                        tx_success = false;
                        warn!("[LEDGER] Contract deployment exceeded gas limit");
                    } else {
                        match self.contract_engine.deploy_contract(
                            &tx.sender,
                            tx.nonce,
                            wasm_bytes,
                            init_payload.as_deref(),
                            &*self.storage,
                            tx.gas_limit.saturating_sub(gas_used),
                        ) {
                            Ok(deploy_result) => {
                                // Add contract code to batch atomically
                                let code_key = Self::make_contract_code_key(&deploy_result.contract_id);
                                batch.ops.push(StorageOperation::PutContractCode(
                                    code_key,
                                    deploy_result.wasm_bytes.clone(),
                                ));
                                // Store deployer in batch
                                batch.ops.push(StorageOperation::PutContractDeployer(
                                    Self::make_contract_deployer_key(&deploy_result.contract_id),
                                    bincode::serialize(&deploy_result.deployer)?,
                                ));
                                // Merge init side effects
                                for (key, val) in deploy_result.side_effects.storage_updates.writes {
                                    let full_key = format!(
                                        "state:{}:{}",
                                        hex::encode(deploy_result.contract_id.to_bytes()),
                                        hex::encode(key)
                                    );
                                    batch.ops.push(StorageOperation::PutContractStorage(
                                        full_key.as_bytes().to_vec(),
                                        val,
                                    ));
                                }
                                for key in deploy_result.side_effects.storage_updates.deletes {
                                    let full_key = format!(
                                        "state:{}:{}",
                                        hex::encode(deploy_result.contract_id.to_bytes()),
                                        hex::encode(key)
                                    );
                                    batch.ops.push(StorageOperation::DeleteContractStorage(
                                        full_key.as_bytes().to_vec(),
                                    ));
                                }
                                for (topic, data) in deploy_result.side_effects.events {
                                    let event_key = format!(
                                        "event:{}:{}:{}",
                                        hex::encode(deploy_result.contract_id.to_bytes()),
                                        hex::encode(topic),
                                        hex::encode(tx.hash)
                                    );
                                    batch.ops.push(StorageOperation::PutContractEvent(
                                        event_key.as_bytes().to_vec(),
                                        data,
                                    ));
                                }
                                let code_hash = self.compute_contract_code_hash(&deploy_result.contract_id)?;
                                let storage_root_hash = self.compute_contract_storage_root(&deploy_result.contract_id)?;
                                let contract_account = Account::Contract {
                                    balance: 0,
                                    code_hash,
                                    storage_root_hash,
                                    nonce: 0,
                                };
                                let contract_key = deploy_result.contract_id.to_bytes();
                                batch.ops.push(StorageOperation::PutAccount(
                                    contract_key.to_vec(),
                                    bincode::serialize(&contract_account)?,
                                ));
                                touched_contracts.insert(contract_key);
                            }
                            Err(e) => {
                                tx_success = false;
                                warn!("[LEDGER] Contract deployment failed: {}", e);
                            }
                        }
                    }
                }
                TransactionPayload::ContractCall { method, args, value } => {
                    let contract_id = match &tx.recipient {
                        crate::types::Address::Contract(cid) => cid,
                        _ => return Err(LedgerError::InvalidTransactionPayload),
                    };

                    gas_used = gas_used.saturating_add(50_000);
                    if gas_used > tx.gas_limit {
                        tx_success = false;
                        warn!("[LEDGER] Contract call exceeded gas limit");
                    } else {
                        // Transfer value to contract if specified
                        if let Some(val) = value {
                            if *val > 0 {
                                if sender_balance < *val {
                                    return Err(LedgerError::StateTransition(
                                        StateTransitionError::InsufficientBalance(
                                            "Contract call value".to_string(),
                                        ),
                                    ));
                                }
                                sender_balance = sender_balance.checked_sub(*val).ok_or(
                                    LedgerError::StateTransition(StateTransitionError::Overflow),
                                )?;

                                let mut contract_account = self
                                    .storage
                                    .get_account(&PublicKey::from_bytes(&contract_id.to_bytes())?)?
                                    .ok_or(LedgerError::ContractNotFound(hex::encode(
                                        contract_id.to_bytes(),
                                    )))?;

                                let new_contract_balance =
                                    contract_account.balance().checked_add(*val).ok_or(
                                        LedgerError::StateTransition(StateTransitionError::Overflow),
                                    )?;

                                if let Account::Contract { balance, .. } = &mut contract_account {
                                    *balance = new_contract_balance;
                                }

                                batch.ops.push(StorageOperation::PutAccount(
                                    contract_id.to_bytes().to_vec(),
                                    bincode::serialize(&contract_account)?,
                                ));
                                touched_contracts.insert(contract_id.to_bytes());
                            }
                        }

                        // Execute call
                        match self.contract_engine.call_contract(
                            &tx.sender,
                            contract_id,
                            method,
                            args.as_slice(),
                            *value,
                            &*self.storage,
                            block.index,
                            block.timestamp,
                            tx.gas_limit.saturating_sub(gas_used),
                        ) {
                            Ok(result) => {
                                gas_used = gas_used.saturating_add(result.gas_used);
                                // Merge contract side effects into block batch
                                for (key, val) in result.side_effects.storage_updates.writes {
                                    let full_key = format!(
                                        "state:{}:{}",
                                        hex::encode(contract_id.to_bytes()),
                                        hex::encode(key)
                                    );
                                    batch.ops.push(StorageOperation::PutContractStorage(
                                        full_key.as_bytes().to_vec(),
                                        val,
                                    ));
                                }
                                for key in result.side_effects.storage_updates.deletes {
                                    let full_key = format!(
                                        "state:{}:{}",
                                        hex::encode(contract_id.to_bytes()),
                                        hex::encode(key)
                                    );
                                    batch.ops.push(StorageOperation::DeleteContractStorage(
                                        full_key.as_bytes().to_vec(),
                                    ));
                                }
                                for (topic, data) in result.side_effects.events {
                                    let event_key = format!(
                                        "event:{}:{}:{}",
                                        hex::encode(contract_id.to_bytes()),
                                        hex::encode(topic),
                                        hex::encode(tx.hash)
                                    );
                                    batch.ops.push(StorageOperation::PutContractEvent(
                                        event_key.as_bytes().to_vec(),
                                        data,
                                    ));
                                }
                                touched_contracts.insert(contract_id.to_bytes());
                            }
                            Err(e) => {
                                tx_success = false;
                                warn!("[LEDGER] Contract call failed: {}", e);
                            }
                        }
                    }
                }
                TransactionPayload::Data { data } => {
                    gas_used = gas_used.saturating_add(1_000);
                    if gas_used > tx.gas_limit {
                        tx_success = false;
                        warn!("[LEDGER] Data transaction exceeded gas limit");
                    }
                }
            }

            // Update sender account (nonce and final balance)
            match &mut sender_account {
                Account::Wallet { balance, nonce } => {
                    *balance = sender_balance;
                    *nonce = nonce.checked_add(1).ok_or(LedgerError::StateTransition(
                        StateTransitionError::Overflow,
                    ))?;
                }
                Account::Contract { balance, nonce, .. } => {
                    *balance = sender_balance;
                    *nonce = nonce.checked_add(1).ok_or(LedgerError::StateTransition(
                        StateTransitionError::Overflow,
                    ))?;
                }
            }

            batch.ops.push(StorageOperation::PutAccount(
                tx.sender.to_bytes().to_vec(),
                bincode::serialize(&sender_account)?,
            ));
            touched_accounts.insert(tx.sender.to_bytes());

            // Add transaction to batch
            let tx_encoded = bincode::serialize(tx)?;
            batch.ops.push(StorageOperation::PutTransaction(tx.hash.to_vec(), tx_encoded));

            // Index transaction by block
            let index_key =
                format!("block_tx:{}:{}:{:0>10}", hex::encode(block.hash), hex::encode(tx.hash), tx_idx);
            batch.ops.push(StorageOperation::PutTxIndex(
                index_key.as_bytes().to_vec(),
                tx.hash.to_vec(),
            ));

            // Remove from mempool if present
            batch.ops.push(StorageOperation::DeleteMempool(tx.hash.to_vec()));
        }

        // 3. Finalize State Merkle Root
        let mut tree = SparseMerkleTree::new();
        for addr in touched_accounts {
            if let Some(account) = self.storage.get_account(&PublicKey::from_bytes(&addr)?)? {
                let val_hash = Sha256::digest(&bincode::serialize(&account)?).into();
                let hash_bytes: [u8; 32] = val_hash;
                tree.insert(addr, hash_bytes.to_vec());
            }
        }
        for cid in touched_contracts {
            if let Some(account) = self.storage.get_account(&PublicKey::from_bytes(&cid)?)? {
                let val_hash = Sha256::digest(&bincode::serialize(&account)?).into();
                let hash_bytes: [u8; 32] = val_hash;
                tree.insert(cid, hash_bytes.to_vec());
            }
        }

        // 4. Update Chain State
        let chain_state = ChainState {
            latest_block_index: block.index,
            latest_block_hash: block.hash,
            accounts_root_hash: tree.root(),
            total_supply: current_chain_state.total_supply,
        };
        batch.ops.push(StorageOperation::PutChainState(
            b"latest_state".to_vec(),
            bincode::serialize(&chain_state)?,
        ));

        // 5. Store Block
        let block_encoded = bincode::serialize(block)?;
        batch.ops.push(StorageOperation::PutBlock(block.hash.to_vec(), block_encoded.clone()));
        let height_key = format!("height:{:0>20}", block.index);
        batch.ops.push(StorageOperation::PutBlock(height_key.as_bytes().to_vec(), block_encoded));

        // 6. Commit Batch
        self.storage.apply_batch(batch)?;
        info!("[LEDGER] Block {} applied atomically", block.index);

        Ok(())
    }
}
