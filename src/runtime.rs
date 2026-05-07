use ed25519_dalek::SigningKey;
use log::{debug, error, info, warn};
use rand::RngCore;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::consensus::{ConsensusEngine, ConsensusError};
use crate::contracts::BaaLSContractEngine;
use crate::contracts::ContractEngine;
use crate::ledger::{Ledger, LedgerError};
use crate::metrics::{time_operation_fn, MetricsCollector};
use crate::storage::{Storage, StorageError};
use crate::sync::SyncLayer;
use crate::types::{Account, Block, ChainState, ContractId, CryptoError, PublicKey, Transaction};

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

#[derive(Debug, Clone)]
pub struct Mempool {
    pub txs_by_hash: HashMap<[u8; 32], Transaction>,
    pub txs_by_sender: HashMap<PublicKey, BTreeMap<u64, [u8; 32]>>,
    pub size_limit: usize,
    pub total_bytes: usize,
    pub ttl_seconds: u64,
    pub max_tx_per_sender: usize,
}

impl Mempool {
    pub fn new(size_limit: usize) -> Self {
        Self::with_ttl(size_limit, 300)
    }

    pub fn with_ttl(size_limit: usize, ttl_seconds: u64) -> Self {
        Self {
            txs_by_hash: HashMap::new(),
            txs_by_sender: HashMap::new(),
            size_limit,
            total_bytes: 0,
            ttl_seconds,
            max_tx_per_sender: 100,
        }
    }

    pub fn insert(&mut self, tx: Transaction) -> Result<(), RuntimeError> {
        if self.txs_by_hash.len() >= self.size_limit {
            self.evict_lowest_priority()?;
        }
        if self.txs_by_hash.contains_key(&tx.hash) {
            return Err(RuntimeError::InvalidTransaction("Duplicate transaction".to_string()));
        }
        // Per-sender rate limiting
        if let Some(map) = self.txs_by_sender.get(&tx.sender) {
            if map.len() >= self.max_tx_per_sender {
                return Err(RuntimeError::InvalidTransaction(format!(
                    "Too many pending transactions from sender: {}",
                    self.max_tx_per_sender
                )));
            }
        }
        let tx_size = std::mem::size_of_val(&tx);
        self.total_bytes += tx_size;
        let sender = tx.sender;
        let nonce = tx.nonce;
        self.txs_by_sender.entry(sender).or_default().insert(nonce, tx.hash);
        self.txs_by_hash.insert(tx.hash, tx);
        Ok(())
    }

    pub fn remove(&mut self, hash: &[u8; 32]) {
        if let Some(tx) = self.txs_by_hash.remove(hash) {
            let tx_size = std::mem::size_of_val(&tx);
            self.total_bytes = self.total_bytes.saturating_sub(tx_size);
            if let Some(map) = self.txs_by_sender.get_mut(&tx.sender) {
                map.remove(&tx.nonce);
                if map.is_empty() {
                    self.txs_by_sender.remove(&tx.sender);
                }
            }
        }
    }

    pub fn get(&self, hash: &[u8; 32]) -> Option<&Transaction> {
        self.txs_by_hash.get(hash)
    }

    pub fn all(&self) -> Vec<&Transaction> {
        self.txs_by_hash.values().collect()
    }

    pub fn sorted_by_priority(&self) -> Vec<&Transaction> {
        let mut txs: Vec<&Transaction> = self.txs_by_hash.values().collect();
        txs.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.timestamp.cmp(&b.timestamp)));
        txs
    }

    pub fn evict_expired(&mut self) -> usize {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let expired_hashes: Vec<[u8; 32]> = self
            .txs_by_hash
            .iter()
            .filter(|(_, tx)| now.saturating_sub(tx.timestamp) > self.ttl_seconds)
            .map(|(h, _)| *h)
            .collect();
        let count = expired_hashes.len();
        for h in &expired_hashes {
            self.remove(h);
        }
        count
    }

    fn evict_lowest_priority(&mut self) -> Result<(), RuntimeError> {
        if self.txs_by_hash.is_empty() {
            return Err(RuntimeError::InvalidTransaction("Mempool full".to_string()));
        }
        let best: Option<[u8; 32]> = self
            .txs_by_hash
            .values()
            .min_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| a.timestamp.cmp(&b.timestamp)) // oldest first
                    .then_with(|| a.gas_limit.cmp(&b.gas_limit)) // least gas first
            })
            .map(|tx| tx.hash);
        if let Some(hash) = best {
            self.remove(&hash);
        }
        if self.txs_by_hash.len() >= self.size_limit {
            return Err(RuntimeError::InvalidTransaction("Mempool full".to_string()));
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.txs_by_hash.clear();
        self.txs_by_sender.clear();
        self.total_bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.txs_by_hash.len()
    }

    pub fn is_empty(&self) -> bool {
        self.txs_by_hash.is_empty()
    }

    pub fn memory_usage(&self) -> usize {
        self.total_bytes
    }
}

pub struct Runtime<S: Storage + 'static, C: ConsensusEngine, Y: SyncLayer> {
    storage: Arc<S>,
    ledger: Arc<Ledger<S, BaaLSContractEngine<S>>>,
    consensus: Arc<C>,
    mempool: Arc<Mutex<Mempool>>,
    chain_state: Arc<Mutex<ChainState>>,
    is_running: Arc<Mutex<bool>>,
    sync_layer: Arc<Y>,
    contract_engine_arc: Arc<BaaLSContractEngine<S>>,
    mempool_size_limit: usize,
    pub auto_block_interval_ms: u64,
    pub auto_block_mempool_threshold: usize,
    pub backup_interval_secs: u64,
    metrics: Arc<MetricsCollector>,
    started_at: Arc<Mutex<Option<SystemTime>>>,
    block_production_shutdown: Arc<Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
    backup_shutdown: Arc<Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
}

impl<S: Storage + 'static, C: ConsensusEngine, Y: SyncLayer> Clone for Runtime<S, C, Y> {
    fn clone(&self) -> Self {
        Self {
            storage: Arc::clone(&self.storage),
            ledger: Arc::clone(&self.ledger),
            consensus: Arc::clone(&self.consensus),
            mempool: Arc::clone(&self.mempool),
            chain_state: Arc::clone(&self.chain_state),
            is_running: Arc::clone(&self.is_running),
            sync_layer: Arc::clone(&self.sync_layer),
            contract_engine_arc: Arc::clone(&self.contract_engine_arc),
            mempool_size_limit: self.mempool_size_limit,
            auto_block_interval_ms: self.auto_block_interval_ms,
            auto_block_mempool_threshold: self.auto_block_mempool_threshold,
            backup_interval_secs: self.backup_interval_secs,
            metrics: Arc::clone(&self.metrics),
            started_at: Arc::clone(&self.started_at),
            block_production_shutdown: Arc::clone(&self.block_production_shutdown),
            backup_shutdown: Arc::clone(&self.backup_shutdown),
        }
    }
}

impl<S: Storage + 'static, C: ConsensusEngine + 'static, Y: SyncLayer + 'static> Runtime<S, C, Y> {
    pub fn new(
        storage: S,
        consensus: C,
        contract_engine: BaaLSContractEngine<S>,
        sync_layer: Y,
    ) -> Result<Self, RuntimeError> {
        debug!("Runtime::new called");
        Self::with_mempool_limit(storage, consensus, contract_engine, sync_layer, 10000)
        // Default limit: 10,000 transactions
    }

    pub fn with_mempool_limit(
        storage: S,
        consensus: C,
        contract_engine: BaaLSContractEngine<S>,
        sync_layer: Y,
        mempool_size_limit: usize,
    ) -> Result<Self, RuntimeError> {
        debug!("Runtime::with_mempool_limit called");
        let storage_arc = Arc::new(storage);
        let contract_engine_arc = Arc::new(contract_engine);
        let ledger =
            Arc::new(Ledger::new(Arc::clone(&storage_arc), Arc::clone(&contract_engine_arc)));

        // Initialize chain if not already initialized
        ledger.initialize_chain()?;

        let initial_chain_state = storage_arc.get_chain_state()?;
        debug!("Initial chain state loaded: {}", initial_chain_state.is_some());
        let initial_chain_state =
            initial_chain_state.ok_or(RuntimeError::ChainInitializationError)?;

        Ok(Runtime {
            storage: storage_arc,
            ledger,
            auto_block_interval_ms: 0, // disabled by default; set before start() to enable
            auto_block_mempool_threshold: 100,
            backup_interval_secs: 0,
            consensus: Arc::new(consensus),
            mempool: Arc::new(Mutex::new(Mempool::new(mempool_size_limit))),
            chain_state: Arc::new(Mutex::new(initial_chain_state)),
            is_running: Arc::new(Mutex::new(false)),
            sync_layer: Arc::new(sync_layer),
            contract_engine_arc,
            mempool_size_limit,
            metrics: Arc::new(MetricsCollector::new()),
            started_at: Arc::new(Mutex::new(None)),
            block_production_shutdown: Arc::new(Mutex::new(None)),
            backup_shutdown: Arc::new(Mutex::new(None)),
        })
    }

    pub fn generate_keypair() -> Result<SigningKey, RuntimeError> {
        // Use random bytes to create a signing key
        let mut secret_key_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut secret_key_bytes);
        Ok(SigningKey::from_bytes(&secret_key_bytes))
    }

    pub fn start(&self) -> Result<(), RuntimeError> {
        let mut is_running = self.is_running.lock().unwrap();
        if *is_running {
            return Err(RuntimeError::AlreadyRunning);
        }
        *is_running = true;
        drop(is_running);

        *self.started_at.lock().unwrap() = Some(SystemTime::now());

        // Reload pending transactions from storage (crash recovery)
        if let Ok(pending) = self.storage.get_pending_transactions() {
            if !pending.is_empty() {
                let mut mempool = self.mempool.lock().unwrap();
                for tx in pending {
                    let _ = mempool.insert(tx);
                }
                info!("Loaded {} pending transactions from storage", mempool.len());
            }
        }

        // Spawn P2P sync listener in background
        let sync_layer = Arc::clone(&self.sync_layer);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .expect("Failed to create sync listener tokio runtime");
            rt.block_on(async move {
                if let Err(e) = sync_layer.start_listener().await {
                    log::error!("Sync listener error: {}", e);
                }
            });
        });

        // Spawn dedicated sync import loop (runs regardless of auto-block setting)
        let self_clone = self.clone();
        std::thread::spawn(move || {
            let rt =
                tokio::runtime::Runtime::new().expect("Failed to create sync import tokio runtime");
            rt.block_on(async move {
                let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(1));
                loop {
                    ticker.tick().await;
                    {
                        let running = self_clone.is_running.lock().unwrap();
                        if !*running {
                            break;
                        }
                    }
                    self_clone.apply_received_blocks();
                    self_clone.sync_with_all_peers();
                }
            });
        });

        // Spawn automated backup if configured
        if self.backup_interval_secs > 0 {
            let (backup_tx, mut backup_rx) = tokio::sync::watch::channel(false);
            *self.backup_shutdown.lock().unwrap() = Some(backup_tx);
            let self_clone = self.clone();
            let interval = self.backup_interval_secs;
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    let mut ticker =
                        tokio::time::interval(tokio::time::Duration::from_secs(interval));
                    loop {
                        tokio::select! {
                            _ = ticker.tick() => {}
                            _ = backup_rx.changed() => {
                                if *backup_rx.borrow() { break; }
                                continue;
                            }
                        }
                        {
                            let running = self_clone.is_running.lock().unwrap();
                            if !*running {
                                break;
                            }
                        }
                        let backup_path = std::path::PathBuf::from(format!(
                            "backup_{}.baals",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs()
                        ));
                        match self_clone.storage().backup_to(&backup_path) {
                            Ok(()) => info!("Automated backup saved to {:?}", backup_path),
                            Err(e) => warn!("Automated backup failed: {}", e),
                        }
                    }
                });
            });
            info!("Automated backup enabled (interval: {}s)", self.backup_interval_secs);
        }

        // Spawn automatic block production if configured
        if self.auto_block_interval_ms > 0 && self.auto_block_mempool_threshold > 0 {
            self.spawn_block_production();
            info!(
                "BaaLS Runtime started (auto-block: {}ms, mempool threshold: {})",
                self.auto_block_interval_ms, self.auto_block_mempool_threshold
            );
        } else {
            info!("BaaLS Runtime started (auto-block disabled)");
        }
        Ok(())
    }

    fn spawn_block_production(&self) {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        *self.block_production_shutdown.lock().unwrap() = Some(shutdown_tx);

        let self_clone = self.clone();
        let interval_ms = self_clone.auto_block_interval_ms;
        let threshold = self_clone.auto_block_mempool_threshold;

        // Use std::thread to ensure we always have a tokio runtime available
        std::thread::spawn(move || {
            let rt =
                tokio::runtime::Runtime::new().expect("Failed to create auto-block tokio runtime");
            rt.block_on(async move {
                let mut ticker =
                    tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));
                loop {
                    tokio::select! {
                        _ = ticker.tick() => {}
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                break;
                            }
                            continue;
                        }
                    }

                    {
                        let running = self_clone.is_running.lock().unwrap();
                        if !*running {
                            break;
                        }
                    }

                    let should_produce = {
                        let mempool = self_clone.mempool.lock().unwrap();
                        !mempool.is_empty() && mempool.len() >= threshold
                    };

                    // Apply any blocks received from peers
                    self_clone.apply_received_blocks();

                    if should_produce {
                        match self_clone.produce_block().await {
                            Ok(block) => debug!(
                                "Auto-produced block #{} ({} txns)",
                                block.index,
                                block.transactions.len()
                            ),
                            Err(e) => {
                                if !matches!(
                                    e,
                                    RuntimeError::ConsensusError(
                                        ConsensusError::NoPendingTransactions
                                    )
                                ) {
                                    warn!("Auto block production error: {}", e);
                                }
                            }
                        }
                    }

                    // Sync with peers to check if we're behind
                    self_clone.sync_with_all_peers();
                }
                debug!("Block production loop terminated");
            });
        });
    }

    pub fn stop(&self) -> Result<(), RuntimeError> {
        let mut is_running = self.is_running.lock().unwrap();
        if !*is_running {
            return Err(RuntimeError::NotRunning);
        }
        *is_running = false;
        drop(is_running);

        // Signal block production to stop
        if let Some(shutdown_tx) = self.block_production_shutdown.lock().unwrap().take() {
            let _ = shutdown_tx.send(true);
        }

        // Signal backup thread to stop
        if let Some(backup_tx) = self.backup_shutdown.lock().unwrap().take() {
            let _ = backup_tx.send(true);
        }

        // Signal P2P listener to stop
        self.sync_layer.stop_listener();

        *self.started_at.lock().unwrap() = None;
        info!("BaaLS Runtime stopped");
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        *self.is_running.lock().unwrap()
    }

    pub fn submit_transaction(&self, transaction: Transaction) -> Result<(), RuntimeError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        let validation_result = time_operation_fn(
            &self.metrics,
            || {
                // 1. Signature verification
                if !transaction.verify_signature()? {
                    return Err(RuntimeError::InvalidTransaction(
                        "Invalid transaction signature".to_string(),
                    ));
                }

                // 2. Timestamp validation: must be within [-60s, +300s] of current time
                if transaction.timestamp < now.saturating_sub(60) {
                    return Err(RuntimeError::InvalidTransaction(
                        "Transaction expired".to_string(),
                    ));
                }
                if transaction.timestamp > now + 300 {
                    return Err(RuntimeError::InvalidTransaction(
                        "Transaction timestamp too far in the future".to_string(),
                    ));
                }

                // 3. Gas limit bounds checking
                if transaction.gas_limit < 21_000 {
                    return Err(RuntimeError::InvalidTransaction(
                        "Gas limit too low (minimum 21,000)".to_string(),
                    ));
                }
                if transaction.gas_limit > 10_000_000 {
                    return Err(RuntimeError::InvalidTransaction(
                        "Gas limit too high (maximum 10,000,000)".to_string(),
                    ));
                }

                // 4. Payload format validation
                match &transaction.payload {
                    crate::types::TransactionPayload::Transfer { amount } => {
                        if *amount == 0 {
                            return Err(RuntimeError::InvalidTransaction(
                                "Transfer amount must be > 0".to_string(),
                            ));
                        }
                    }
                    crate::types::TransactionPayload::ContractDeploy {
                        wasm_bytes,
                        init_payload: _,
                    } => {
                        if wasm_bytes.len() < 8 {
                            return Err(RuntimeError::InvalidTransaction(
                                "WASM bytecode too short".to_string(),
                            ));
                        }
                        // Validate WASM magic bytes
                        if wasm_bytes[0..4] != [0x00, 0x61, 0x73, 0x6d] {
                            return Err(RuntimeError::InvalidTransaction(
                                "Invalid WASM magic bytes".to_string(),
                            ));
                        }
                    }
                    crate::types::TransactionPayload::ContractCall {
                        method,
                        args: _,
                        value: _,
                    } => {
                        if method.is_empty() {
                            return Err(RuntimeError::InvalidTransaction(
                                "Method name cannot be empty".to_string(),
                            ));
                        }
                        if method.len() > 256 {
                            return Err(RuntimeError::InvalidTransaction(
                                "Method name too long".to_string(),
                            ));
                        }
                    }
                    crate::types::TransactionPayload::Data { data } => {
                        if data.len() > 1024 * 1024 {
                            // 1MB max
                            return Err(RuntimeError::InvalidTransaction(
                                "Data payload too large (max 1MB)".to_string(),
                            ));
                        }
                    }
                }

                // 5. Nonce validation against chain state + mempool
                let sender_pk = transaction.sender;
                let sender_account = self
                    .storage
                    .get_account(&sender_pk)?
                    .unwrap_or(Account::Wallet { balance: 0, nonce: 0 });

                let mempool = self.mempool.lock().unwrap();
                let highest_mempool_nonce = mempool
                    .txs_by_sender
                    .get(&sender_pk)
                    .and_then(|map| map.keys().max().cloned())
                    .unwrap_or(sender_account.nonce());
                drop(mempool);

                let expected_nonce = highest_mempool_nonce + 1;
                if transaction.nonce < expected_nonce {
                    return Err(RuntimeError::InvalidTransaction(format!(
                        "Invalid nonce: expected at least {}, got {}",
                        expected_nonce, transaction.nonce
                    )));
                }
                Ok(())
            },
            MetricsCollector::record_transaction_validation,
        );
        validation_result?;

        let hash = transaction.hash;

        // Evict expired transactions before checking capacity
        {
            let mut mempool = self.mempool.lock().unwrap();
            let expired = mempool.evict_expired();
            if expired > 0 {
                debug!("Evicted {} expired transactions from mempool", expired);
            }
        }

        let mut mempool = self.mempool.lock().unwrap();
        mempool.insert(transaction)?;
        // Persist to storage for crash recovery
        if let Err(e) = self.storage.put_pending_transaction(mempool.get(&hash).unwrap()) {
            warn!("Failed to persist pending tx {}: {}", crate::types::format_hex(&hash), e);
        }
        self.metrics.record_mempool_operation();
        info!("Transaction submitted: {}", crate::types::format_hex(&hash));
        Ok(())
    }

    fn produce_block_sync(&self) -> Result<Block, RuntimeError> {
        debug!("[PRODUCE_BLOCK] Starting block production");

        debug!("[PRODUCE_BLOCK] Acquiring mempool lock");
        let mut mempool = self.mempool.lock().unwrap();
        debug!("[PRODUCE_BLOCK] Mempool lock acquired, checking if empty");

        if mempool.is_empty() {
            debug!("[PRODUCE_BLOCK] Mempool is empty, returning error");
            return Err(ConsensusError::NoPendingTransactions.into());
        }

        debug!("[PRODUCE_BLOCK] Mempool has {} transactions", mempool.len());

        debug!("[PRODUCE_BLOCK] Acquiring mutable chain state lock");
        let mut current_chain_state = self.chain_state.lock().unwrap();
        debug!("[PRODUCE_BLOCK] Mutable chain state lock acquired");

        debug!("[PRODUCE_BLOCK] Getting previous block from storage");
        let prev_block = self
            .storage
            .get_block(&current_chain_state.latest_block_hash)?
            .ok_or(StorageError::NotFound)?;
        debug!(
            "[PRODUCE_BLOCK] Previous block retrieved: index={}, hash={}",
            prev_block.index,
            crate::types::format_hex(&prev_block.hash)
        );

        debug!("[PRODUCE_BLOCK] Collecting transactions from mempool (priority-ordered)");
        mempool.evict_expired();
        let mut transactions: Vec<Transaction> =
            mempool.sorted_by_priority().iter().map(|tx| (*tx).clone()).collect();
        transactions.sort_by_key(|tx| (tx.sender, tx.nonce));

        // Filter to only include txs with continuous nonces per sender.
        // Gap transactions (nonce > next_expected) are kept in mempool for later blocks.
        let mut sender_next: HashMap<PublicKey, u64> = HashMap::new();
        transactions.retain(|tx| {
            let expected = sender_next.entry(tx.sender).or_insert_with(|| {
                self.storage
                    .get_account(&tx.sender)
                    .ok()
                    .flatten()
                    .map(|a| a.nonce() + 1)
                    .unwrap_or(1)
            });
            if tx.nonce == *expected {
                *expected = tx.nonce + 1;
                true
            } else if tx.nonce > *expected {
                // Gap transaction — keep in mempool, skip for this block
                false
            } else {
                // Stale nonce — should have been rejected earlier, skip
                false
            }
        });

        debug!("[PRODUCE_BLOCK] Collected {} transactions", transactions.len());

        debug!("[PRODUCE_BLOCK] Calling consensus.generate_block");
        let new_block =
            self.consensus.generate_block(&transactions, &prev_block, &current_chain_state)?;
        debug!(
            "[PRODUCE_BLOCK] Block generated: index={}, hash={}",
            new_block.index,
            crate::types::format_hex(&new_block.hash)
        );

        // Release mempool lock before processing block
        debug!("[PRODUCE_BLOCK] Releasing mempool lock");
        drop(mempool);

        debug!("[PRODUCE_BLOCK] Starting block processing with ledger");
        let processing_result: Result<(), RuntimeError> = time_operation_fn(
            &self.metrics,
            || {
                debug!("[PRODUCE_BLOCK] Validating block with ledger");
                // Validate and apply block to ledger
                self.ledger.validate_block(&new_block, &current_chain_state)?;
                debug!("[PRODUCE_BLOCK] Block validation successful");

                debug!("[PRODUCE_BLOCK] Applying block to ledger");
                // Pass contract_engine to apply_block
                self.ledger.apply_block(new_block.clone(), &mut current_chain_state)?;
                debug!("[PRODUCE_BLOCK] Block application successful");
                Ok(())
            },
            MetricsCollector::record_block_processing,
        );

        processing_result?;
        info!(
            "Block #{} produced ({} txns, {} gas)",
            new_block.index,
            new_block.transactions.len(),
            new_block.transactions.iter().map(|t| t.gas_limit).sum::<u64>()
        );

        // Update metrics
        self.metrics.update_average_block_size(std::mem::size_of_val(&new_block));

        println!("Block produced and applied: {}", crate::types::format_hex(&new_block.hash));
        debug!("[PRODUCE_BLOCK] Block production completed successfully");

        // Optionally broadcast the new block
        debug!("[PRODUCE_BLOCK] Starting async broadcast task");
        let sync_layer_clone = Arc::clone(&self.sync_layer);
        let new_block_clone = new_block.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                debug!("[BROADCAST] Starting broadcast in spawned task");
                let peers = sync_layer_clone.discover_peers().await.unwrap_or_else(|e| {
                    error!("[BROADCAST] Error discovering peers: {}", e);
                    Vec::new()
                });
                debug!("[BROADCAST] Discovered {} peers", peers.len());
                if let Err(e) = sync_layer_clone.broadcast_block(&new_block_clone, &peers).await {
                    error!("[BROADCAST] Error broadcasting block: {}", e);
                } else {
                    debug!("[BROADCAST] Block broadcast completed successfully");
                }
            });
        } else {
            std::thread::spawn(move || {
                if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    rt.block_on(async move {
                        debug!("[BROADCAST] Starting broadcast in fallback task");
                        let peers = sync_layer_clone.discover_peers().await.unwrap_or_else(|e| {
                            error!("[BROADCAST] Error discovering peers: {}", e);
                            Vec::new()
                        });
                        debug!("[BROADCAST] Discovered {} peers", peers.len());
                        if let Err(e) =
                            sync_layer_clone.broadcast_block(&new_block_clone, &peers).await
                        {
                            error!("[BROADCAST] Error broadcasting block: {}", e);
                        } else {
                            debug!("[BROADCAST] Block broadcast completed successfully");
                        }
                    });
                }
            });
        }
        debug!("[PRODUCE_BLOCK] Broadcast task spawned");

        // Remove only the transactions that were included in the block
        debug!("[PRODUCE_BLOCK] Removing included transactions from mempool");
        let mut mempool = self.mempool.lock().unwrap();
        for tx in &new_block.transactions {
            mempool.remove(&tx.hash);
            if let Err(e) = self.storage.remove_pending_transaction(&tx.hash) {
                warn!(
                    "Failed to remove persisted pending tx {}: {}",
                    crate::types::format_hex(&tx.hash),
                    e
                );
            }
        }
        debug!(
            "[PRODUCE_BLOCK] Removed {} transactions from mempool",
            new_block.transactions.len()
        );

        debug!("[PRODUCE_BLOCK] Returning produced block");
        Ok(new_block)
    }

    pub async fn produce_block(&self) -> Result<Block, RuntimeError> {
        self.produce_block_sync()
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
        let mut secret_key_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut secret_key_bytes);
        Ok(SigningKey::from_bytes(&secret_key_bytes))
    }

    pub fn get_current_timestamp(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    pub fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, RuntimeError> {
        self.storage.get_block_by_height(height).map_err(RuntimeError::StorageError)
    }

    pub fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, RuntimeError> {
        self.storage.get_account(address).map_err(RuntimeError::StorageError)
    }

    pub fn contract_storage_read(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        self.storage.contract_storage_read(contract_id, key).map_err(RuntimeError::StorageError)
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    pub fn sync_layer(&self) -> &Y {
        &self.sync_layer
    }

    pub fn ledger(&self) -> &Ledger<S, BaaLSContractEngine<S>> {
        &self.ledger
    }

    pub fn chain_state_lock(&self) -> &Mutex<ChainState> {
        &self.chain_state
    }

    pub fn create_account(
        &self,
        address: &PublicKey,
        account: Account,
    ) -> Result<(), RuntimeError> {
        self.storage.put_account(address, &account).map_err(RuntimeError::StorageError)
    }

    pub fn get_mempool(&self) -> Result<Vec<Transaction>, RuntimeError> {
        Ok(self.mempool.lock().unwrap().all().into_iter().cloned().collect())
    }

    pub fn get_transaction_history(
        &self,
        address: &PublicKey,
        limit: usize,
    ) -> Result<Vec<Transaction>, RuntimeError> {
        // Query confirmed transactions from storage
        let mut history = self
            .storage
            .get_transactions_by_address(address, limit)
            .map_err(RuntimeError::StorageError)?;

        // Supplement with pending mempool transactions
        if history.len() < limit {
            let mempool = self.mempool.lock().unwrap();
            for tx in mempool.all().iter() {
                if tx.sender == *address
                    || (matches!(tx.recipient, crate::types::Address::Wallet(pk) if pk == *address))
                {
                    if history.len() >= limit {
                        break;
                    }
                    history.push((*tx).clone());
                }
            }
        }

        Ok(history)
    }

    pub fn get_block_by_hash(&self, hash: &[u8; 32]) -> Result<Option<Block>, RuntimeError> {
        self.storage.get_block(hash).map_err(RuntimeError::StorageError)
    }

    /// Reorganize the chain by applying a sequence of blocks from a fork.
    /// Validates each block for continuity and correctness before applying.
    /// Returns the new chain height after reorganization.
    pub fn reorganize_chain(&self, fork_blocks: &[Block]) -> Result<u64, RuntimeError> {
        if fork_blocks.is_empty() {
            return Ok(self.get_chain_state()?.latest_block_index);
        }

        let mut current_chain_state = self.chain_state.lock().unwrap();
        let local_height = current_chain_state.latest_block_index;
        let fork_height = fork_blocks.last().map(|b| b.index).unwrap_or(0);

        if fork_height <= local_height {
            info!(
                "[CHAIN] Fork not longer, skipping (local={}, fork={})",
                local_height, fork_height
            );
            return Ok(local_height);
        }

        info!(
            "[CHAIN] Reorganizing from {} to {} ({} blocks)",
            local_height + 1,
            fork_height,
            fork_blocks.len()
        );

        let mut expected_index = current_chain_state.latest_block_index + 1;
        let mut expected_prev_hash = current_chain_state.latest_block_hash;

        for block in fork_blocks {
            if block.index != expected_index {
                return Err(RuntimeError::InvalidTransaction(format!(
                    "Fork block index mismatch: expected {}, got {}",
                    expected_index, block.index
                )));
            }
            if block.prev_hash != expected_prev_hash {
                return Err(RuntimeError::InvalidTransaction(format!(
                    "Fork block prev_hash mismatch at index {}",
                    block.index
                )));
            }

            self.ledger.validate_block(block, &current_chain_state)?;
            self.ledger.apply_block(block.clone(), &mut current_chain_state)?;

            expected_index = block.index + 1;
            expected_prev_hash = block.hash;
        }

        info!("[CHAIN] Reorganized to height {}", current_chain_state.latest_block_index);
        Ok(current_chain_state.latest_block_index)
    }

    pub fn get_node_status(&self) -> Result<ChainState, RuntimeError> {
        self.get_chain_state()
    }

    pub fn add_peer(&self, address: &str) -> Result<(), RuntimeError> {
        self.sync_layer
            .add_peer_by_address(address)
            .map_err(|e| RuntimeError::InvalidTransaction(format!("Add peer failed: {}", e)))
    }

    /// Apply blocks received from peers via the sync layer.
    fn apply_received_blocks(&self) {
        let blocks = self.sync_layer.poll_received_blocks();
        if blocks.is_empty() {
            return;
        }
        info!("[SYNC] Processing {} blocks received from peers", blocks.len());
        for block in blocks {
            match self.ledger.validate_block(&block, &self.chain_state.lock().unwrap()) {
                Ok(()) => {
                    let mut chain_state = self.chain_state.lock().unwrap();
                    match self.ledger.apply_block(block.clone(), &mut chain_state) {
                        Ok(()) => {
                            info!(
                                "[SYNC] Applied received block #{} ({} txns)",
                                block.index,
                                block.transactions.len()
                            );
                        }
                        Err(e) => {
                            warn!("[SYNC] Failed to apply received block #{}: {}", block.index, e);
                        }
                    }
                }
                Err(e) => {
                    warn!("[SYNC] Received block #{} failed validation: {}", block.index, e);
                }
            }
        }
    }

    /// Check all known peers and sync with any that are ahead.
    fn sync_with_all_peers(&self) {
        let sync_layer = Arc::clone(&self.sync_layer);
        let self_clone = self.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create sync tokio runtime");
            rt.block_on(async move {
                let peers = match sync_layer.discover_peers().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                if peers.is_empty() {
                    return;
                }
                let chain_state = self_clone.chain_state.lock().unwrap().clone();
                for peer in &peers {
                    match sync_layer.sync_with_peer(peer, &chain_state).await {
                        Ok(_block) => {
                            debug!("[SYNC] Synced with peer {}", peer.address);
                            self_clone.apply_received_blocks();
                        }
                        Err(e) => {
                            if !matches!(e, crate::sync::SyncError::SynchronizationError(_)) {
                                debug!("[SYNC] Sync with {} failed: {}", peer.address, e);
                            }
                        }
                    }
                }
            });
        });
    }

    pub fn get_metrics(&self) -> Result<HashMap<String, f64>, RuntimeError> {
        Ok(self.metrics.get_summary())
    }

    pub fn get_detailed_metrics(&self) -> Result<crate::metrics::PerformanceMetrics, RuntimeError> {
        Ok(self.metrics.get_metrics())
    }

    pub fn get_mempool_stats(&self) -> Result<MempoolStats, RuntimeError> {
        let mempool = self.mempool.lock().unwrap();
        let mut priority_counts = std::collections::HashMap::new();
        let mut total_gas_limit = 0;
        let mut total_size = 0;

        for tx in mempool.all() {
            *priority_counts.entry(tx.priority).or_insert(0) += 1;
            total_gas_limit += tx.gas_limit;
            total_size += std::mem::size_of_val(tx);
        }

        Ok(MempoolStats {
            total_transactions: mempool.len(),
            max_size: self.mempool_size_limit,
            total_gas_limit,
            total_size,
            priority_distribution: priority_counts,
        })
    }

    /// Deploy a smart contract
    pub fn deploy_contract(
        &self,
        deployer: &PublicKey,
        wasm_bytes: &[u8],
        init_payload: Option<&[u8]>,
        gas_limit: u64,
    ) -> Result<ContractId, RuntimeError> {
        info!(
            "[RUNTIME] Deploying contract from deployer: {}",
            hex::encode(deployer.to_bytes())
        );

        // Get deployer's current nonce for deterministic contract ID
        let deployer_account = self.storage.get_account(deployer)?;
        let deployer_nonce = deployer_account.as_ref().map(|a| a.nonce()).unwrap_or(0);

        // Use the contract engine to deploy the contract
        let contract_id = self
            .contract_engine_arc
            .deploy_contract(
                deployer,
                deployer_nonce,
                wasm_bytes,
                init_payload,
                &*self.storage,
                gas_limit,
            )
            .map_err(|e| {
                RuntimeError::InvalidTransaction(format!("Contract deployment failed: {}", e))
            })?;

        info!(
            "[RUNTIME] Contract deployed successfully with ID: {}",
            hex::encode(contract_id.to_bytes())
        );
        Ok(contract_id)
    }

    /// Call a smart contract method
    pub fn call_contract(
        &self,
        caller: &PublicKey,
        contract_id: &ContractId,
        method_name: &str,
        args: &[Vec<u8>],
        value: Option<u64>,
        _gas_limit: u64,
    ) -> Result<Vec<u8>, RuntimeError> {
        info!(
            "[RUNTIME] Calling contract {} method '{}' from caller: {}",
            hex::encode(contract_id.to_bytes()),
            method_name,
            hex::encode(caller.to_bytes())
        );

        // Use the contract engine to call the contract
        let result = self
            .contract_engine_arc
            .call_contract(caller, contract_id, method_name, args, value, &*self.storage, 0, 0)
            .map_err(|e| {
                RuntimeError::InvalidTransaction(format!("Contract call failed: {}", e))
            })?;

        info!("[RUNTIME] Contract call completed successfully");
        Ok(result)
    }

    /// Query a smart contract (read-only)
    pub fn query_contract(
        &self,
        contract_id: &ContractId,
        method_name: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, RuntimeError> {
        info!(
            "[RUNTIME] Querying contract {} method '{}'",
            hex::encode(contract_id.to_bytes()),
            method_name
        );

        // Use the contract engine to query the contract
        let result = self
            .contract_engine_arc
            .query_contract(contract_id, method_name, payload, &*self.storage, 0, 0)
            .map_err(|e| {
                RuntimeError::InvalidTransaction(format!("Contract query failed: {}", e))
            })?;

        info!("[RUNTIME] Contract query completed successfully");
        Ok(result)
    }

    /// Get pending transactions from mempool
    pub fn get_pending_transactions(&self) -> Result<Vec<Transaction>, RuntimeError> {
        let mempool = self.mempool.lock().unwrap();
        Ok(mempool.all().iter().map(|tx| (*tx).clone()).collect())
    }

    /// Get comprehensive node status information
    pub fn get_comprehensive_node_status(&self) -> Result<RuntimeNodeStatus, RuntimeError> {
        let chain_state = self.get_chain_state()?;
        let mempool_stats = self.get_mempool_stats()?;
        let metrics = self.get_detailed_metrics()?;
        let running = *self.is_running.lock().unwrap();
        let start_time = *self.started_at.lock().unwrap();
        let uptime_seconds = if running {
            start_time
                .and_then(|started_at| SystemTime::now().duration_since(started_at).ok())
                .map_or(0, |duration| duration.as_secs())
        } else {
            0
        };

        Ok(RuntimeNodeStatus {
            running,
            chain_state,
            mempool_stats,
            metrics,
            uptime_seconds,
            peer_count: self.sync_layer.peer_count(),
        })
    }

    pub fn get_health_status(&self) -> Result<crate::metrics::HealthStatus, RuntimeError> {
        let mut health = self.metrics.health_check();
        let chain_state = self.get_chain_state()?;
        health.latest_block_index = chain_state.latest_block_index;
        health.latest_block_hash = hex::encode(chain_state.latest_block_hash);
        health.mempool_size = self.get_pending_transactions()?.len();
        health.connected_peers = self.sync_layer.peer_count();
        Ok(health)
    }
}

#[derive(Debug, Clone)]
pub struct MempoolStats {
    pub total_transactions: usize,
    pub max_size: usize,
    pub total_gas_limit: u64,
    pub total_size: usize,
    pub priority_distribution: std::collections::HashMap<u8, usize>,
}

#[derive(Debug, Clone)]
pub struct RuntimeNodeStatus {
    pub running: bool,
    pub chain_state: ChainState,
    pub mempool_stats: MempoolStats,
    pub metrics: crate::metrics::PerformanceMetrics,
    pub uptime_seconds: u64,
    pub peer_count: usize,
}
