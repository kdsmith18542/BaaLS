use ed25519_dalek::SigningKey;
use log::{debug, error, info, warn};
use rand::RngCore;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
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

const MAX_NONCE_GAP_SKIP_CYCLES: u32 = 3;

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
    pub max_tx_per_sender_per_second: usize,
    pub sender_timestamps: HashMap<PublicKey, Vec<u64>>,
    /// Tracks how many block-production cycles a transaction has been skipped
    /// due to a nonce gap. Evicted after MAX_SKIP_CYCLES consecutive skips.
    pub skip_count: HashMap<[u8; 32], u32>,
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
            max_tx_per_sender_per_second: 10,
            sender_timestamps: HashMap::new(),
            skip_count: HashMap::new(),
        }
    }

    pub fn insert(&mut self, tx: Transaction) -> Result<(), RuntimeError> {
        if self.txs_by_hash.len() >= self.size_limit {
            self.evict_lowest_priority()?;
        }
        if self.txs_by_hash.contains_key(&tx.hash) {
            return Err(RuntimeError::InvalidTransaction("Duplicate transaction".to_string()));
        }
        // Per-sender count-based rate limiting
        if let Some(map) = self.txs_by_sender.get(&tx.sender) {
            if map.len() >= self.max_tx_per_sender {
                return Err(RuntimeError::InvalidTransaction(format!(
                    "Too many pending transactions from sender: {}",
                    self.max_tx_per_sender
                )));
            }
        }
        // Per-sender time-windowed rate limiting
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let window = 1; // 1 second window
        let entries = self.sender_timestamps.entry(tx.sender).or_default();
        entries.retain(|&t| now.saturating_sub(t) < window);
        if entries.len() >= self.max_tx_per_sender_per_second {
            return Err(RuntimeError::InvalidTransaction(format!(
                "Rate limit exceeded for sender: max {} tx/s",
                self.max_tx_per_sender_per_second
            )));
        }
        entries.push(now);
        let tx_size = tx.payload_size_estimate() + std::mem::size_of::<Transaction>();
        self.total_bytes = self.total_bytes.saturating_add(tx_size);
        let sender = tx.sender;
        let nonce = tx.nonce;
        self.skip_count.entry(tx.hash).or_insert(0);
        self.txs_by_sender.entry(sender).or_default().insert(nonce, tx.hash);
        self.txs_by_hash.insert(tx.hash, tx);
        Ok(())
    }

    pub fn remove(&mut self, hash: &[u8; 32]) {
        if let Some(tx) = self.txs_by_hash.remove(hash) {
            let tx_size = tx.payload_size_estimate() + std::mem::size_of::<Transaction>();
            self.total_bytes = self.total_bytes.saturating_sub(tx_size);
            if let Some(map) = self.txs_by_sender.get_mut(&tx.sender) {
                map.remove(&tx.nonce);
                if map.is_empty() {
                    self.txs_by_sender.remove(&tx.sender);
                }
            }
            self.skip_count.remove(hash);
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
        self.sender_timestamps.clear();
        self.skip_count.clear();
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
    sync_in_flight: Arc<AtomicBool>,
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
            sync_in_flight: Arc::clone(&self.sync_in_flight),
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
            sync_in_flight: Arc::new(AtomicBool::new(false)),
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

    /// Override per-sender mempool limits for high-throughput scenarios (e.g. benchmarks/tests).
    pub fn configure_mempool_sender_limits(
        &self,
        max_tx_per_sender: usize,
        max_tx_per_sender_per_second: usize,
    ) {
        let mut mempool = self.mempool.lock().unwrap();
        mempool.max_tx_per_sender = max_tx_per_sender;
        mempool.max_tx_per_sender_per_second = max_tx_per_sender_per_second;
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
        // Gap transactions (nonce > next_expected) are tracked for eviction:
        // after MAX_NONCE_GAP_SKIP_CYCLES consecutive skips they are removed
        // to prevent nonce-gap DoS.
        let mut sender_next: HashMap<PublicKey, u64> = HashMap::new();
        let mut to_evict: Vec<[u8; 32]> = Vec::new();
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
                // Gap transaction — increment skip counter, evict if exceeded
                let count = mempool.skip_count.entry(tx.hash).or_insert(0);
                *count += 1;
                if *count > MAX_NONCE_GAP_SKIP_CYCLES {
                    to_evict.push(tx.hash);
                    warn!(
                        "[MEMPOOL] Evicting tx {} from sender {} after {} nonce-gap skips",
                        hex::encode(tx.hash),
                        hex::encode(tx.sender.to_bytes()),
                        *count
                    );
                    false
                } else {
                    false
                }
            } else {
                // Stale nonce — should have been rejected earlier, skip
                false
            }
        });
        for hash in &to_evict {
            mempool.remove(hash);
        }

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
                self.ledger.validate_block(&new_block)?;
                debug!("[PRODUCE_BLOCK] Block validation successful");

                debug!("[PRODUCE_BLOCK] Applying block to ledger");
                self.ledger.apply_block(&new_block)?;
                debug!("[PRODUCE_BLOCK] Block application successful");
                Ok(())
            },
            MetricsCollector::record_block_processing,
        );

        processing_result?;

        // Reload chain state from storage after block application
        if let Ok(Some(new_state)) = self.storage.get_chain_state() {
            *current_chain_state = new_state;
        }

        info!(
            "Block #{} produced ({} txns, {} gas)",
            new_block.index,
            new_block.transactions.len(),
            new_block.transactions.iter().map(|t| t.gas_limit).sum::<u64>()
        );

        // Update metrics (use serialized size estimate for accuracy)
        let block_size: usize = std::mem::size_of::<Block>()
            + new_block
                .transactions
                .iter()
                .map(|tx| tx.payload_size_estimate() + std::mem::size_of::<Transaction>())
                .sum::<usize>();
        self.metrics.update_average_block_size(block_size);

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

    pub fn refresh_chain_state(&self) -> Result<(), RuntimeError> {
        if let Ok(Some(state)) = self.storage.get_chain_state() {
            *self.chain_state.lock().unwrap() = state;
        }
        Ok(())
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

        let chain_snapshot = self.chain_state.lock().unwrap().clone();
        let local_height = chain_snapshot.latest_block_index;
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

        let mut expected_index = chain_snapshot.latest_block_index + 1;
        let mut expected_prev_hash = chain_snapshot.latest_block_hash;

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

            if let Err(e) = self.consensus.validate_block(block, &chain_snapshot) {
                return Err(RuntimeError::InvalidTransaction(format!(
                    "Consensus validation failed on fork block #{}: {}",
                    block.index, e
                )));
            }

            self.ledger.validate_block(block)?;
            self.ledger.apply_block(block)?;

            expected_index = block.index + 1;
            expected_prev_hash = block.hash;
        }

        if let Ok(Some(new_state)) = self.storage.get_chain_state() {
            let new_height = new_state.latest_block_index;
            *self.chain_state.lock().unwrap() = new_state;
            info!("[CHAIN] Reorganized to height {}", new_height);
            Ok(new_height)
        } else {
            Ok(chain_snapshot.latest_block_index)
        }
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

        // Check if the first block's prev_hash matches our latest block
        let latest_hash = self.chain_state.lock().unwrap().latest_block_hash;
        if let Some(first_block) = blocks.first() {
            if first_block.prev_hash != latest_hash {
                log::info!(
                    "[RUNTIME] Fork detected — reorganizing chain with {} fork blocks",
                    blocks.len()
                );
                match self.reorganize_chain(&blocks) {
                    Ok(new_height) => {
                        info!("[RUNTIME] Chain reorganized to height {}", new_height);
                    }
                    Err(e) => {
                        warn!("[RUNTIME] Chain reorganization failed: {}", e);
                    }
                }
                return;
            }
        }

        // Normal sequential block application.
        for block in blocks {
            // Scope for consensus validation lock
            let consensus_valid = {
                let chain_state = self.chain_state.lock().unwrap();
                self.consensus.validate_block(&block, &chain_state).is_ok()
            };
            if !consensus_valid {
                let chain_state = self.chain_state.lock().unwrap();
                warn!(
                    "[SYNC] Consensus validation failed for block #{} against chain at height {}",
                    block.index, chain_state.latest_block_index
                );
                continue;
            }

            // Validate state transition
            if let Err(e) = self.ledger.validate_block(&block) {
                warn!("[SYNC] Received block #{} failed validation: {}", block.index, e);
                continue;
            }

            // Apply block
            match self.ledger.apply_block(&block) {
                Ok(()) => {
                    info!(
                        "[SYNC] Applied received block #{} ({} txns)",
                        block.index,
                        block.transactions.len()
                    );
                    // Reload chain state from storage after block application
                    if let Ok(Some(new_state)) = self.storage.get_chain_state() {
                        *self.chain_state.lock().unwrap() = new_state;
                    }
                }
                Err(e) => {
                    warn!("[SYNC] Failed to apply received block #{}: {}", block.index, e);
                }
            }
        }
    }

    /// Check all known peers and sync with any that are ahead.
    fn sync_with_all_peers(&self) {
        if self
            .sync_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            debug!("[SYNC] Skipping sync tick because a sync worker is already in flight");
            return;
        }

        let sync_layer = Arc::clone(&self.sync_layer);
        let self_clone = self.clone();

        // Spawn on the current tokio runtime instead of creating a new one per tick
        tokio::runtime::Handle::current().spawn(async move {
            let peers = match sync_layer.discover_peers().await {
                Ok(p) => p,
                Err(_) => {
                    self_clone.sync_in_flight.store(false, Ordering::Release);
                    return;
                }
            };
            if peers.is_empty() {
                self_clone.sync_in_flight.store(false, Ordering::Release);
                return;
            }
            for peer in &peers {
                // Re-read chain state after each peer to avoid stale comparisons
                let chain_state = self_clone.chain_state.lock().unwrap().clone();
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
            self_clone.sync_in_flight.store(false, Ordering::Release);
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
        let mut priority_counts: std::collections::HashMap<u8, usize> =
            std::collections::HashMap::new();
        let mut total_gas_limit: u64 = 0;
        let mut total_size: usize = 0;

        for tx in mempool.all() {
            let count = priority_counts.entry(tx.priority).or_insert(0);
            *count = (*count).saturating_add(1);
            total_gas_limit = total_gas_limit.saturating_add(tx.gas_limit);
            total_size = total_size
                .saturating_add(tx.payload_size_estimate() + std::mem::size_of::<Transaction>());
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
        let deploy_result = self
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

        // Store contract code atomically (immutable, idempotent)
        self.storage
            .put_contract_code(&deploy_result.contract_id, &deploy_result.wasm_bytes)
            .map_err(|e| {
                RuntimeError::InvalidTransaction(format!("Failed to store contract code: {}", e))
            })?;
        self.storage
            .put_contract_deployer(&deploy_result.contract_id, &deploy_result.deployer)
            .map_err(|e| {
                RuntimeError::InvalidTransaction(format!("Failed to store deployer: {}", e))
            })?;

        // Apply init side effects to storage
        for (key, val) in deploy_result.side_effects.storage_updates.writes {
            self.storage.contract_storage_write(&deploy_result.contract_id, &key, &val).map_err(
                |e| {
                    RuntimeError::InvalidTransaction(format!("Failed to write init storage: {}", e))
                },
            )?;
        }
        for key in deploy_result.side_effects.storage_updates.deletes {
            self.storage.contract_storage_remove(&deploy_result.contract_id, &key).map_err(
                |e| {
                    RuntimeError::InvalidTransaction(format!(
                        "Failed to delete init storage: {}",
                        e
                    ))
                },
            )?;
        }

        info!(
            "[RUNTIME] Contract deployed successfully with ID: {}",
            hex::encode(deploy_result.contract_id.to_bytes())
        );
        Ok(deploy_result.contract_id)
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

        // Use real block context from current chain state
        let chain_state = self.get_chain_state()?;
        let block_index = chain_state.latest_block_index;
        let block_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let gas = if _gas_limit > 0 { _gas_limit } else { 1_000_000 };
        let result = self
            .contract_engine_arc
            .call_contract(
                caller,
                contract_id,
                method_name,
                args,
                value,
                &*self.storage,
                block_index,
                block_timestamp,
                gas,
            )
            .map_err(|e| {
                RuntimeError::InvalidTransaction(format!("Contract call failed: {}", e))
            })?;

        info!("[RUNTIME] Contract call completed successfully");
        Ok(result.output)
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

        // Use real block context from current chain state
        let chain_state = self.get_chain_state()?;
        let block_index = chain_state.latest_block_index;
        let block_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let result = self
            .contract_engine_arc
            .query_contract(
                contract_id,
                method_name,
                payload,
                &*self.storage,
                block_index,
                block_timestamp,
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Address, Transaction, TransactionPayload, TransactionSignature};
    use ed25519_dalek::SigningKey;
    use rand::RngCore;

    fn make_key() -> (SigningKey, PublicKey) {
        let mut sk_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut sk_bytes);
        let sk = SigningKey::from_bytes(&sk_bytes);
        let pk = PublicKey::from(sk.verifying_key());
        (sk, pk)
    }

    fn dummy_tx(sender: PublicKey, nonce: u64, timestamp: u64) -> Transaction {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender,
            nonce,
            timestamp,
            recipient: Address::Wallet(sender),
            payload: TransactionPayload::Data { data: vec![] },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            gas_price: 0,
            priority: 0,
            metadata: None,
        };
        tx.hash = tx.calculate_hash().unwrap();
        tx
    }

    #[test]
    fn test_mempool_rate_limiting_count() {
        let mut mempool = Mempool::new(1000);
        mempool.max_tx_per_sender = 3;
        let (_, pk) = make_key();

        assert!(mempool.insert(dummy_tx(pk, 1, 100)).is_ok());
        assert!(mempool.insert(dummy_tx(pk, 2, 101)).is_ok());
        assert!(mempool.insert(dummy_tx(pk, 3, 102)).is_ok());
        assert!(mempool.insert(dummy_tx(pk, 4, 103)).is_err());
        assert_eq!(mempool.len(), 3);
    }

    #[test]
    fn test_mempool_rate_limiting_time_window() {
        let mut mempool = Mempool::new(1000);
        mempool.max_tx_per_sender_per_second = 2;
        let (_, pk) = make_key();
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();

        assert!(mempool.insert(dummy_tx(pk, 1, now)).is_ok());
        assert!(mempool.insert(dummy_tx(pk, 2, now)).is_ok());
        assert!(mempool.insert(dummy_tx(pk, 3, now)).is_err());

        let (_, pk2) = make_key();
        assert!(mempool.insert(dummy_tx(pk2, 1, now)).is_ok());
    }

    #[test]
    fn test_mempool_duplicate_rejected() {
        let mut mempool = Mempool::new(1000);
        let (_, pk) = make_key();
        let mut tx = dummy_tx(pk, 1, 100);
        tx.hash = tx.calculate_hash().unwrap();

        assert!(mempool.insert(tx.clone()).is_ok());
        assert!(mempool.insert(tx).is_err());
    }

    #[test]
    fn test_mempool_eviction() {
        let mut mempool = Mempool::with_ttl(2, 1);
        let (_, pk) = make_key();

        assert!(mempool.insert(dummy_tx(pk, 1, 100)).is_ok());
        assert!(mempool.insert(dummy_tx(pk, 2, 101)).is_ok());

        let (_, pk2) = make_key();
        assert!(mempool.insert(dummy_tx(pk2, 1, 102)).is_ok());
        assert_eq!(mempool.len(), 2);
    }
}
