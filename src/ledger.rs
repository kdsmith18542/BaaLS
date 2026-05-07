use log::{debug, info, warn};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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
    #[allow(dead_code)]
    contract_engine: Arc<C>,
}

impl<S: Storage, C: ContractEngine> Ledger<S, C> {
    const MAX_FUTURE_BLOCK_TIMESTAMP_SECONDS: u64 = 10;
    pub fn new(storage: Arc<S>, contract_engine: Arc<C>) -> Self {
        debug!("Ledger::new called");
        Ledger { storage, contract_engine }
    }

    pub fn initialize_chain(&self) -> Result<(), LedgerError> {
        debug!("Ledger::initialize_chain called");
        // Check if chain state already exists
        let chain_state_exists = self.storage.get_chain_state()?.is_some();
        debug!("Chain state exists before init: {}", chain_state_exists);
        if chain_state_exists {
            return Ok(());
        }

        // Create a genesis block
        let genesis_block = Block {
            index: 0,
            timestamp: 0,
            prev_hash: [0; 32], // Genesis block has no previous hash
            hash: [0; 32],      // Will be calculated after creation
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
            total_supply: 0,             // No native token for now
        };

        let mut batch = StorageBatch::default();
        batch.ops.push(StorageOperation::PutBlock(
            bincode::serialize(&genesis_block.hash)?,
            bincode::serialize(&genesis_block)?,
        ));
        batch.ops.push(StorageOperation::PutChainState(
            "global:current".as_bytes().to_vec(),
            bincode::serialize(&initial_chain_state)?,
        ));

        self.storage.apply_batch(batch)?;
        debug!(
            "Chain initialized with genesis block: {}",
            crate::types::format_hex(&genesis_block.hash)
        );
        Ok(())
    }

    pub fn validate_block(
        &self,
        block: &Block,
        current_chain_state: &ChainState,
    ) -> Result<(), LedgerError> {
        info!("[LEDGER] Starting block validation");
        info!(
            "[LEDGER] Block: index={}, hash={}",
            block.index,
            crate::types::format_hex(&block.hash)
        );
        info!(
            "[LEDGER] Current chain state: latest_index={}, latest_hash={}",
            current_chain_state.latest_block_index,
            crate::types::format_hex(&current_chain_state.latest_block_hash)
        );

        // Basic Block Header Validation
        info!("[LEDGER] Validating block index");
        if block.index != current_chain_state.latest_block_index + 1 {
            return Err(LedgerError::BlockValidation(format!(
                "Invalid block index: expected {}, got {}",
                current_chain_state.latest_block_index + 1,
                block.index
            )));
        }

        info!("[LEDGER] Validating previous hash");
        if block.prev_hash != current_chain_state.latest_block_hash {
            return Err(LedgerError::BlockValidation(format!(
                "Invalid previous hash: expected {:x?}, got {:x?}",
                current_chain_state.latest_block_hash, block.prev_hash
            )));
        }

        info!("[LEDGER] Validating block hash");
        let calculated_hash = block.calculate_hash()?;
        if calculated_hash != block.hash {
            return Err(LedgerError::BlockValidation(format!(
                "Invalid block hash: expected {:x?}, got {:x?}",
                calculated_hash, block.hash
            )));
        }

        // Timestamp check (simplified for MVP, typically more robust logic needed)
        info!("[LEDGER] Validating block timestamp");
        if block.index > 0 {
            let prev_block =
                self.storage.get_block(&block.prev_hash)?.ok_or(LedgerError::NotFound)?;
            if block.timestamp <= prev_block.timestamp {
                return Err(LedgerError::BlockValidation(
                    "Block timestamp is not greater than previous block's timestamp".to_string(),
                ));
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| LedgerError::BlockValidation(format!("System time error: {}", e)))?
            .as_secs();
        if block.timestamp > now + Self::MAX_FUTURE_BLOCK_TIMESTAMP_SECONDS {
            return Err(LedgerError::BlockValidation(
                "Block timestamp too far in the future".to_string(),
            ));
        }

        // Transaction Validation (within the block) - only basic checks for MVP
        info!("[LEDGER] Validating {} transactions", block.transactions.len());
        for (i, tx) in block.transactions.iter().enumerate() {
            info!(
                "[LEDGER] Validating transaction {}: hash={}",
                i,
                crate::types::format_hex(&tx.hash)
            );
            if !tx.verify_signature()? {
                return Err(LedgerError::BlockValidation(format!(
                    "Invalid signature for transaction: {:x?}",
                    tx.hash
                )));
            }
            // Further transaction validation (nonce, balance) will happen during state transition
        }

        info!("[LEDGER] Block validation completed successfully");
        Ok(())
    }

    fn contract_id_to_account_key(contract_id: &ContractId) -> Result<PublicKey, LedgerError> {
        for salt in 0u32..4096 {
            let mut hasher = Sha256::new();
            hasher.update(contract_id.to_bytes());
            hasher.update(salt.to_le_bytes());
            let candidate: [u8; 32] = hasher.finalize().into();
            if let Ok(pk) = PublicKey::from_bytes(&candidate) {
                return Ok(pk);
            }
        }

        Err(LedgerError::StateTransition(StateTransitionError::ContractError(
            "Failed to derive deterministic contract account key".to_string(),
        )))
    }

    fn compute_contract_code_hash(
        &self,
        contract_id: &ContractId,
    ) -> Result<[u8; 32], LedgerError> {
        let code = self.storage.get_contract_code(contract_id)?.ok_or_else(|| {
            LedgerError::ContractNotFound(format!(
                "Contract not found: {}",
                hex::encode(contract_id.to_bytes())
            ))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&code);
        Ok(hasher.finalize().into())
    }

    fn compute_contract_storage_root(
        &self,
        contract_id: &ContractId,
    ) -> Result<[u8; 32], LedgerError> {
        let mut keys = self.storage.get_all_contract_storage_keys(contract_id)?;
        if keys.is_empty() {
            return Ok([0; 32]);
        }
        keys.sort();
        keys.dedup();

        let mut smt = SparseMerkleTree::new();
        for key in keys {
            if let Some(value) = self.storage.contract_storage_read(contract_id, &key)? {
                // Hash the storage key to get a 32-byte SMT key
                let smt_key = sha2::Sha256::digest(&key).into();
                smt.insert(smt_key, value);
            }
        }

        if smt.is_empty() {
            Ok([0; 32])
        } else {
            Ok(smt.root())
        }
    }

    pub fn apply_block(
        &self,
        mut block: Block,
        current_chain_state: &mut ChainState,
    ) -> Result<(), LedgerError> {
        info!("[LEDGER] Starting block application");
        info!(
            "[LEDGER] Block: index={}, hash={}, transactions={}",
            block.index,
            crate::types::format_hex(&block.hash),
            block.transactions.len()
        );

        let mut batch = StorageBatch::default();
        let mut accounts_to_update: BTreeMap<PublicKey, Account> = BTreeMap::new();
        let mut touched_contracts: BTreeSet<[u8; 32]> = BTreeSet::new();

        info!("[LEDGER] Processing {} transactions", block.transactions.len());
        // Sort transactions by (sender, nonce) to ensure sequential nonce processing
        block.transactions.sort_by_key(|tx| (tx.sender, tx.nonce));
        for (i, tx) in block.transactions.iter().enumerate() {
            info!(
                "[LEDGER] Processing transaction {}: hash={}",
                i,
                crate::types::format_hex(&tx.hash)
            );

            let sender_pk = tx.sender;
            let mut sender_account =
                if let Some(updated_account) = accounts_to_update.get(&sender_pk) {
                    updated_account.clone()
                } else {
                    self.storage.get_account(&sender_pk)?.ok_or_else(|| {
                        LedgerError::StateTransition(StateTransitionError::AccountNotFound(
                            format!("Sender account not found: {:?}", sender_pk),
                        ))
                    })?
                };
            info!("[LEDGER] Sender account found: nonce={}", sender_account.nonce());

            // Nonce Check
            if sender_account.nonce() + 1 != tx.nonce {
                return Err(LedgerError::StateTransition(StateTransitionError::InvalidNonce {
                    expected: sender_account.nonce() + 1,
                    got: tx.nonce,
                }));
            }
            sender_account.set_nonce(sender_account.nonce() + 1);
            accounts_to_update.insert(sender_pk, sender_account.clone());
            info!("[LEDGER] Sender nonce updated to {}", sender_account.nonce());

            let mut gas_used = 0u64;
            let mut tx_success = true;

            match &tx.payload {
                TransactionPayload::Transfer { amount } => {
                    info!("[LEDGER] Processing transfer transaction: amount={}", amount);
                    // Fixed gas cost for native transfer
                    let transfer_gas_cost = 21000;
                    gas_used += transfer_gas_cost;
                    if gas_used > tx.gas_limit {
                        tx_success = false;
                        warn!("[LEDGER] Transaction exceeded gas limit");
                    } else {
                        // Get the sender account from our updates
                        if let Some(Account::Wallet { balance, .. }) =
                            accounts_to_update.get_mut(&tx.sender)
                        {
                            if *balance < *amount {
                                return Err(LedgerError::StateTransition(
                                    StateTransitionError::InsufficientBalance(format!(
                                        "{:?}",
                                        tx.sender
                                    )),
                                ));
                            }
                            *balance -= *amount;
                            info!("[LEDGER] Sender balance reduced to {}", *balance);
                        } else {
                            return Err(LedgerError::StateTransition(
                                StateTransitionError::InvalidPayload,
                            ));
                        }

                        // Handle recipient
                        match tx.recipient {
                            crate::types::Address::Wallet(recipient_pk) => {
                                let mut recipient_account = if let Some(updated_account) =
                                    accounts_to_update.get(&recipient_pk)
                                {
                                    updated_account.clone()
                                } else if let Some(existing_account) =
                                    self.storage.get_account(&recipient_pk)?
                                {
                                    existing_account
                                } else {
                                    Account::Wallet { balance: 0, nonce: 0 }
                                };

                                if let Account::Wallet { balance, .. } = &mut recipient_account {
                                    *balance += amount;
                                    info!("[LEDGER] Recipient balance increased to {}", *balance);
                                } else {
                                    return Err(LedgerError::StateTransition(
                                        StateTransitionError::InvalidPayload,
                                    ));
                                }
                                accounts_to_update.insert(recipient_pk, recipient_account);
                            }
                            crate::types::Address::Contract(_) => {
                                return Err(LedgerError::StateTransition(
                                    StateTransitionError::InvalidPayload,
                                ));
                            }
                        }
                    }
                }
                TransactionPayload::ContractDeploy { wasm_bytes, init_payload } => {
                    gas_used += 100_000;
                    if gas_used > tx.gas_limit {
                        tx_success = false;
                        warn!("[LEDGER] Contract deployment exceeded gas limit");
                    } else {
                        // Deploy the contract via the contract engine
                        let deployer = tx.sender;
                        let deployer_nonce = accounts_to_update
                            .get(&deployer)
                            .map(|a| a.nonce())
                            .unwrap_or(sender_account.nonce());
                        match self.contract_engine.deploy_contract(
                            &deployer,
                            deployer_nonce,
                            wasm_bytes,
                            init_payload.as_deref(),
                            &*self.storage,
                            tx.gas_limit.saturating_sub(gas_used),
                        ) {
                            Ok(cid) => {
                                info!(
                                    "[LEDGER] Contract deployed with ID: {}",
                                    hex::encode(cid.to_bytes())
                                );
                                let contract_key = Self::contract_id_to_account_key(&cid)?;
                                let code_hash = self.compute_contract_code_hash(&cid)?;
                                let storage_root_hash = self.compute_contract_storage_root(&cid)?;
                                let contract_nonce = match accounts_to_update.get(&contract_key) {
                                    Some(Account::Contract { nonce, .. }) => *nonce,
                                    Some(Account::Wallet { .. }) => {
                                        return Err(LedgerError::StateTransition(
                                            StateTransitionError::ContractError(
                                                "Contract account key collides with wallet account"
                                                    .to_string(),
                                            ),
                                        ));
                                    }
                                    None => self
                                        .storage
                                        .get_account(&contract_key)?
                                        .and_then(|account| match account {
                                            Account::Contract { nonce, .. } => Some(nonce),
                                            Account::Wallet { .. } => None,
                                        })
                                        .unwrap_or(0),
                                };
                                accounts_to_update.insert(
                                    contract_key,
                                    Account::Contract {
                                        code_hash,
                                        storage_root_hash,
                                        nonce: contract_nonce,
                                    },
                                );
                                touched_contracts.insert(cid.to_bytes());
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

                    if self.storage.get_contract_code(contract_id)?.is_none() {
                        return Err(LedgerError::ContractNotFound(format!(
                            "Contract not found: {}",
                            hex::encode(contract_id.to_bytes())
                        )));
                    }

                    gas_used += 50_000;
                    if gas_used > tx.gas_limit {
                        tx_success = false;
                        warn!("[LEDGER] Contract call exceeded gas limit");
                    } else {
                        // Handle native token value transfer to contract
                        if let Some(transfer_amount) = value {
                            if *transfer_amount > 0 {
                                if let Some(Account::Wallet { balance, .. }) =
                                    accounts_to_update.get_mut(&tx.sender)
                                {
                                    if *balance < *transfer_amount {
                                        return Err(LedgerError::StateTransition(
                                            StateTransitionError::InsufficientBalance(format!(
                                                "{:?}",
                                                tx.sender
                                            )),
                                        ));
                                    }
                                    *balance -= *transfer_amount;
                                }
                            }
                        }

                        // Execute the contract via the engine
                        match self.contract_engine.call_contract(
                            &tx.sender,
                            contract_id,
                            method,
                            args,
                            *value,
                            &*self.storage,
                            block.index,
                            block.timestamp,
                        ) {
                            Ok(result) => {
                                info!(
                                    "[LEDGER] Contract call to {}::{} returned {} bytes",
                                    hex::encode(contract_id.to_bytes()),
                                    method,
                                    result.len()
                                );
                                touched_contracts.insert(contract_id.to_bytes());
                            }
                            Err(e) => {
                                tx_success = false;
                                warn!(
                                    "[LEDGER] Contract call to {}::{} failed: {}",
                                    hex::encode(contract_id.to_bytes()),
                                    method,
                                    e
                                );
                            }
                        }
                    }
                }
                TransactionPayload::Data { data: _ } => {
                    // For MVP, just allow storing data. No specific state changes yet.
                    gas_used += 10_000;
                    if gas_used > tx.gas_limit {
                        tx_success = false;
                        warn!("[LEDGER] Data transaction exceeded gas limit");
                    } else {
                        info!("[LEDGER] Data transaction processed successfully");
                    }
                }
            }

            // If transaction failed due to out of gas, skip state changes for this tx
            if !tx_success {
                return Err(LedgerError::StateTransition(StateTransitionError::ExecutionFailed(
                    format!("Transaction execution failed: {}", crate::types::format_hex(&tx.hash)),
                )));
            }

            // Remove from mempool after successful processing
            batch.ops.push(StorageOperation::DeleteMempool(tx.hash.to_vec()));
        }

        // Recompute and update storage roots for all contracts touched in this block.
        for contract_id_bytes in touched_contracts {
            let contract_id = ContractId::from_bytes(&contract_id_bytes);
            let contract_key = Self::contract_id_to_account_key(&contract_id)?;
            let storage_root_hash = self.compute_contract_storage_root(&contract_id)?;

            let current_account = if let Some(account) = accounts_to_update.get(&contract_key) {
                account.clone()
            } else {
                self.storage.get_account(&contract_key)?.unwrap_or(Account::Contract {
                    code_hash: self.compute_contract_code_hash(&contract_id)?,
                    storage_root_hash: [0; 32],
                    nonce: 0,
                })
            };

            match current_account {
                Account::Contract { code_hash, nonce, .. } => {
                    accounts_to_update.insert(
                        contract_key,
                        Account::Contract { code_hash, storage_root_hash, nonce },
                    );
                }
                Account::Wallet { .. } => {
                    return Err(LedgerError::StateTransition(StateTransitionError::ContractError(
                        "Contract account key collides with wallet account".to_string(),
                    )));
                }
            }
        }

        // Apply account updates
        for (address, account) in &accounts_to_update {
            batch.ops.push(StorageOperation::PutAccount(
                address.to_bytes().to_vec(),
                bincode::serialize(account)?,
            ));
        }

        // Build merged view of all accounts (existing + updated)
        let mut merged_accounts: BTreeMap<PublicKey, Account> = BTreeMap::new();
        let all_existing_accounts = self.storage.get_all_accounts()?;
        for (pk, account) in all_existing_accounts {
            merged_accounts.insert(pk, account);
        }
        for (pk, account) in &accounts_to_update {
            merged_accounts.insert(*pk, account.clone());
        }

        // Calculate sparse Merkle root keyed by account-address hash.
        let mut merkle = SparseMerkleTree::new();
        for (pk, account) in &merged_accounts {
            let key_hash: [u8; 32] = Sha256::digest(pk.to_bytes()).into();
            merkle.insert(key_hash, bincode::serialize(account)?);
        }
        let accounts_root_hash = if merkle.is_empty() { [0; 32] } else { merkle.root() };

        // Update chain state
        current_chain_state.latest_block_hash = block.hash;
        current_chain_state.latest_block_index = block.index;
        current_chain_state.accounts_root_hash = accounts_root_hash;
        batch.ops.push(StorageOperation::PutChainState(
            "global:current".as_bytes().to_vec(),
            bincode::serialize(current_chain_state)?,
        ));

        // Store the block itself
        batch
            .ops
            .push(StorageOperation::PutBlock(block.hash.to_vec(), bincode::serialize(&block)?));

        // Store transactions and index them by block hash
        for (i, tx) in block.transactions.iter().enumerate() {
            // Store the transaction
            batch
                .ops
                .push(StorageOperation::PutTransaction(tx.hash.to_vec(), bincode::serialize(tx)?));
            // Index the transaction by block (add to batch)
            let index_key =
                format!("block_tx:{}:{}:{:0>10}", hex::encode(block.hash), hex::encode(tx.hash), i);
            batch.ops.push(StorageOperation::PutTxIndex(
                index_key.as_bytes().to_vec(),
                tx.hash.to_vec(),
            ));
        }

        self.storage.apply_batch(batch)?;
        info!("[LEDGER] Block application completed successfully");
        Ok(())
    }
}
