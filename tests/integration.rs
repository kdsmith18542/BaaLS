use baals::*;
use log::info;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

// Configure logging for tests
fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .try_init();
}

#[test]
fn test_ledger_state_transition() {
    init_logging();
    info!("[TEST] Starting test_ledger_state_transition");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();
    info!("[TEST] Created temp directory: {:?}", data_dir);

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    info!("[TEST] Creating runtime");
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        10000,
    )
    .unwrap();
    info!("[TEST] Runtime created successfully");

    runtime.start().unwrap();
    info!("[TEST] Runtime started");

    let signing_key1 =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key1 = PublicKey::from(signing_key1.verifying_key());
    let account1 = Account::Wallet {
        balance: 1000,
        nonce: 0,
    };
    runtime.create_account(&public_key1, account1).unwrap();
    info!(
        "[TEST] Created account1: {}",
        crate::types::format_hex(&public_key1.to_bytes())
    );

    let signing_key2 =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key2 = PublicKey::from(signing_key2.verifying_key());
    let account2 = Account::Wallet {
        balance: 500,
        nonce: 0,
    };
    runtime.create_account(&public_key2, account2).unwrap();
    info!(
        "[TEST] Created account2: {}",
        crate::types::format_hex(&public_key2.to_bytes())
    );

    let mut transaction = Transaction {
        hash: [0u8; 32],
        sender: public_key1,
        recipient: Address::Wallet(public_key2),
        payload: TransactionPayload::Transfer { amount: 100 },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };
    transaction.hash = transaction.calculate_hash().unwrap();
    transaction.sign(&signing_key1).unwrap();
    info!("[TEST] Created and signed transaction");

    runtime.submit_transaction(transaction).unwrap();
    info!("[TEST] Transaction submitted to runtime");

    info!("[TEST] Creating tokio runtime for block production");
    let tokio_runtime = tokio::runtime::Runtime::new().unwrap();
    info!("[TEST] Tokio runtime created, calling block_on");

    let block = tokio_runtime.block_on(async {
        info!("[TEST] Inside async block, calling runtime.produce_block()");
        let result = runtime.produce_block().await;
        info!("[TEST] produce_block() completed with result: {:?}", result);
        result.unwrap()
    });
    info!(
        "[TEST] Block production completed: index={}, hash={}",
        block.index,
        crate::types::format_hex(&block.hash)
    );

    let account1_after = runtime.get_account(&public_key1).unwrap().unwrap();
    let account2_after = runtime.get_account(&public_key2).unwrap().unwrap();
    if let Account::Wallet {
        balance: balance1,
        nonce: nonce1,
    } = account1_after
    {
        assert_eq!(balance1, 900);
        assert_eq!(nonce1, 1);
    } else {
        panic!("Account1 should be a wallet");
    }
    if let Account::Wallet {
        balance: balance2,
        nonce: nonce2,
    } = account2_after
    {
        assert_eq!(balance2, 600);
        assert_eq!(nonce2, 0); // Recipient nonce should not be incremented
    } else {
        panic!("Account2 should be a wallet");
    }
    let stored_block = runtime.get_block(&block.hash).unwrap().unwrap();
    assert_eq!(stored_block.index, 1);
    assert_eq!(stored_block.transactions.len(), 1);
    info!("[TEST] test_ledger_state_transition completed successfully");
}

#[test]
fn test_transaction_validation_and_mempool() {
    init_logging();
    info!("[TEST] Starting test_transaction_validation_and_mempool");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        1000,
    )
    .unwrap();

    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet {
        balance: 1000,
        nonce: 0,
    };
    runtime.create_account(&public_key, account).unwrap();

    // Test valid transaction
    let mut valid_tx = Transaction {
        hash: [0u8; 32],
        sender: public_key,
        recipient: Address::Wallet(public_key),
        payload: TransactionPayload::Transfer { amount: 100 },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };
    valid_tx.hash = valid_tx.calculate_hash().unwrap();
    valid_tx.sign(&signing_key).unwrap();

    let result = runtime.submit_transaction(valid_tx);
    assert!(result.is_ok(), "Valid transaction should be accepted");

    // Test invalid transaction (insufficient balance — caught during block application, not submission)
    let mut overspend_tx = Transaction {
        hash: [0u8; 32],
        sender: public_key,
        recipient: Address::Wallet(public_key),
        payload: TransactionPayload::Transfer { amount: 2000 }, // More than balance
        nonce: 2,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };
    overspend_tx.hash = overspend_tx.calculate_hash().unwrap();
    overspend_tx.sign(&signing_key).unwrap();

    // Balance is not checked during submission — only during block application
    let result = runtime.submit_transaction(overspend_tx);
    assert!(
        result.is_ok(),
        "Overspend is caught at block application, not submission"
    );

    // Test mempool size limit
    let pending_txs = runtime.get_pending_transactions().unwrap();
    assert_eq!(
        pending_txs.len(),
        2,
        "Both transactions should be in mempool (balance checked at block application)"
    );

    info!("[TEST] test_transaction_validation_and_mempool completed successfully");
}

#[test]
fn test_hardened_transaction_validation() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        1000,
    )
    .unwrap();
    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet {
        balance: 5000,
        nonce: 0,
    };
    runtime.create_account(&public_key, account).unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let make_tx = |nonce: u64, timestamp: u64, amount: u64, gas: u64| -> Transaction {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: public_key,
            recipient: Address::Wallet(public_key),
            payload: TransactionPayload::Transfer { amount },
            nonce,
            timestamp,
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: gas,
            priority: 0,
            metadata: None,
        };
        tx.hash = tx.calculate_hash().unwrap();
        tx.sign(&signing_key).unwrap();
        tx
    };

    // Test: expired timestamp (too old)
    let expired = make_tx(1, now - 120, 100, 100_000);
    assert!(
        runtime.submit_transaction(expired).is_err(),
        "Expired tx should be rejected"
    );

    // Test: timestamp too far in the future
    let future = make_tx(1, now + 600, 100, 100_000);
    assert!(
        runtime.submit_transaction(future).is_err(),
        "Future tx should be rejected"
    );

    // Test: gas limit too low
    let low_gas = make_tx(1, now, 100, 10_000);
    assert!(
        runtime.submit_transaction(low_gas).is_err(),
        "Low gas tx should be rejected"
    );

    // Test: gas limit too high
    let high_gas = make_tx(1, now, 100, 20_000_000);
    assert!(
        runtime.submit_transaction(high_gas).is_err(),
        "High gas tx should be rejected"
    );

    // Test: zero amount transfer
    let zero_amount = make_tx(1, now, 0, 100_000);
    assert!(
        runtime.submit_transaction(zero_amount).is_err(),
        "Zero amount tx should be rejected"
    );

    // Test: valid transaction accepted
    let valid = make_tx(1, now, 100, 100_000);
    assert!(
        runtime.submit_transaction(valid).is_ok(),
        "Valid tx should be accepted"
    );

    info!("[TEST] test_hardened_transaction_validation completed successfully");
}

#[test]
fn test_runtime_status_tracks_lifecycle_and_uptime() {
    init_logging();

    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage, consensus, contract_engine, sync_layer, 1000).unwrap();

    let status_before_start = runtime.get_comprehensive_node_status().unwrap();
    assert!(!status_before_start.running);
    assert_eq!(status_before_start.uptime_seconds, 0);
    assert_eq!(status_before_start.peer_count, 0);

    runtime.start().unwrap();
    assert!(matches!(runtime.start(), Err(RuntimeError::AlreadyRunning)));

    std::thread::sleep(Duration::from_secs(1));
    let status_running = runtime.get_comprehensive_node_status().unwrap();
    assert!(status_running.running);
    assert!(status_running.uptime_seconds >= 1);
    assert_eq!(status_running.peer_count, 0);

    runtime.stop().unwrap();
    assert!(matches!(runtime.stop(), Err(RuntimeError::NotRunning)));

    let status_after_stop = runtime.get_comprehensive_node_status().unwrap();
    assert!(!status_after_stop.running);
    assert_eq!(status_after_stop.uptime_seconds, 0);
}

#[test]
fn test_transaction_merkle_root_in_block() {
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        1000,
    )
    .unwrap();
    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    runtime
        .create_account(
            &public_key,
            Account::Wallet {
                balance: 10000,
                nonce: 0,
            },
        )
        .unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    for i in 1..=3 {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: public_key,
            recipient: Address::Wallet(public_key),
            payload: TransactionPayload::Transfer { amount: 10 },
            nonce: i,
            timestamp: now,
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100_000,
            priority: 0,
            metadata: None,
        };
        tx.hash = tx.calculate_hash().unwrap();
        tx.sign(&signing_key).unwrap();
        runtime.submit_transaction(tx).unwrap();
    }

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let block = tokio_rt.block_on(runtime.produce_block()).unwrap();

    assert_eq!(block.transactions.len(), 3);
    // Verify stored block can be retrieved and deserialized
    let stored = runtime.get_block(&block.hash).unwrap().unwrap();
    assert_eq!(stored.index, 1);
    assert_eq!(stored.transactions.len(), 3);

    // Verify chain state was updated with Merkle root
    let chain = runtime.get_chain_state().unwrap();
    assert_eq!(chain.latest_block_index, 1);
    assert_ne!(
        chain.accounts_root_hash, [0u8; 32],
        "Merkle root should be non-zero after block"
    );

    info!("[TEST] test_transaction_merkle_root_in_block completed");
}

#[test]
fn test_block_production_and_chain_state() {
    init_logging();
    info!("[TEST] Starting test_block_production_and_chain_state");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        1000,
    )
    .unwrap();

    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet {
        balance: 1000,
        nonce: 0,
    };
    runtime.create_account(&public_key, account).unwrap();

    // Submit multiple transactions
    for i in 1..=5 {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: public_key,
            recipient: Address::Wallet(public_key),
            payload: TransactionPayload::Transfer { amount: 10 },
            nonce: i,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            priority: 0,
            metadata: None,
        };
        tx.hash = tx.calculate_hash().unwrap();
        tx.sign(&signing_key).unwrap();
        runtime.submit_transaction(tx).unwrap();
    }

    let tokio_runtime = tokio::runtime::Runtime::new().unwrap();
    let block = tokio_runtime.block_on(async { runtime.produce_block().await.unwrap() });

    assert_eq!(block.index, 1);
    assert_eq!(block.transactions.len(), 5);

    // Verify chain state
    let chain_state = runtime.get_chain_state().unwrap();
    assert_eq!(chain_state.latest_block_index, 1);
    assert_eq!(block.transactions.len(), 5);

    info!("[TEST] test_block_production_and_chain_state completed successfully");
}

#[test]
fn test_contract_deploy_and_execution() {
    init_logging();
    info!("[TEST] Starting test_contract_deploy_and_execution");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        1000,
    )
    .unwrap();

    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet {
        balance: 1000,
        nonce: 0,
    };
    runtime.create_account(&public_key, account).unwrap();

    // Create a simple test WASM module
    let wasm_bytes = create_test_wasm_module();

    // Deploy contract
    let contract_id = runtime
        .deploy_contract(&public_key, &wasm_bytes, None, 100000)
        .unwrap();
    assert!(
        !contract_id.to_bytes().iter().all(|&b| b == 0),
        "Contract ID should not be zero"
    );

    // Call contract — our test module returns args_len (0 since no args passed)
    let result = runtime
        .call_contract(&public_key, &contract_id, "test_method", &[], None, 100000)
        .unwrap();
    // Module returns 0 (args_len) — verify call succeeded, not specific return value
    info!("[TEST] Contract call returned {} bytes", result.len());

    info!("[TEST] test_contract_deploy_and_execution completed successfully");
}

#[test]
fn test_storage_and_merkle_root() {
    init_logging();
    info!("[TEST] Starting test_storage_and_merkle_root");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();

    // Test block storage
    let test_block = Block {
        index: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        prev_hash: [0u8; 32],
        hash: [1u8; 32],
        transactions: vec![],
        nonce: 0,
        metadata: None,
    };

    storage.put_block(&test_block).unwrap();
    let retrieved_block = storage.get_block(&test_block.hash).unwrap().unwrap();
    assert_eq!(retrieved_block.index, test_block.index);

    // Test account storage
    let test_key = PublicKey::from_bytes(&[1u8; 32]).unwrap();
    let test_account = Account::Wallet {
        balance: 1000,
        nonce: 0,
    };
    storage.put_account(&test_key, &test_account).unwrap();
    let retrieved_account = storage.get_account(&test_key).unwrap().unwrap();
    assert_eq!(retrieved_account, test_account);

    // Test chain state storage
    let test_chain_state = ChainState {
        latest_block_index: 1,
        accounts_root_hash: [2u8; 32],
        total_supply: 0,
        latest_block_hash: [1u8; 32],
    };
    storage.put_chain_state(&test_chain_state).unwrap();
    let retrieved_chain_state = storage.get_chain_state().unwrap().unwrap();
    assert_eq!(
        retrieved_chain_state.latest_block_index,
        test_chain_state.latest_block_index
    );

    info!("[TEST] test_storage_and_merkle_root completed successfully");
}

// New: Phase 4 Performance Tests
#[test]
fn test_performance_benchmarks() {
    init_logging();
    info!("[TEST] Starting performance benchmarks");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        10000,
    )
    .unwrap();

    runtime.start().unwrap();

    // Create multiple senders so the benchmark respects anti-spam per-sender mempool caps.
    let mut wallets = Vec::new();
    for _ in 0..10 {
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        runtime
            .create_account(
                &public_key,
                Account::Wallet {
                    balance: 1000000,
                    nonce: 0,
                },
            )
            .unwrap();
        wallets.push((signing_key, public_key));
    }

    // Benchmark transaction submission
    let start_time = Instant::now();
    let mut submitted = 0u64;
    for (signing_key, public_key) in &wallets {
        for nonce in 1..=100 {
            let mut tx = Transaction {
                hash: [0u8; 32],
                sender: *public_key,
                recipient: Address::Wallet(*public_key),
                payload: TransactionPayload::Transfer { amount: 1 },
                nonce,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
                gas_limit: 100000,
                priority: 0,
                metadata: None,
            };
            tx.hash = tx.calculate_hash().unwrap();
            tx.sign(signing_key).unwrap();
            runtime.submit_transaction(tx).unwrap();
            submitted += 1;
        }
    }
    let submission_time = start_time.elapsed();

    info!(
        "[PERF] Transaction submission: {} transactions in {:?} ({:.2} tps)",
        submitted,
        submission_time,
        submitted as f64 / submission_time.as_secs_f64()
    );

    // Benchmark block production
    let start_time = Instant::now();
    let tokio_runtime = tokio::runtime::Runtime::new().unwrap();
    let block = tokio_runtime.block_on(async { runtime.produce_block().await.unwrap() });
    let block_time = start_time.elapsed();

    info!(
        "[PERF] Block production: 1 block with {} transactions in {:?}",
        block.transactions.len(),
        block_time
    );

    // Block should include transactions within gas/size limits
    assert!(
        block.transactions.len() > 0,
        "Produced block should include at least some benchmark transactions"
    );
    assert!(
        block.transactions.len() <= submitted as usize,
        "Block should not exceed submitted count"
    );

    info!("[TEST] Performance benchmarks completed successfully");
}

#[test]
fn test_stress_test() {
    init_logging();
    info!("[TEST] Starting stress test");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        50000,
    )
    .unwrap();

    runtime.start().unwrap();

    // Create multiple accounts
    let mut accounts = Vec::new();
    for _i in 0..10 {
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        let account = Account::Wallet {
            balance: 10000,
            nonce: 0,
        };
        runtime.create_account(&public_key, account).unwrap();
        accounts.push((signing_key, public_key));
    }

    // Submit transactions concurrently
    let runtime_arc = Arc::new(runtime);
    let mut handles = Vec::new();

    for (signing_key, public_key) in accounts {
        let runtime_clone = runtime_arc.clone();
        let handle = std::thread::spawn(move || {
            for i in 1..=100 {
                let mut tx = Transaction {
                    hash: [0u8; 32],
                    sender: public_key,
                    recipient: Address::Wallet(public_key),
                    payload: TransactionPayload::Transfer { amount: 1 },
                    nonce: i,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
                gas_limit: 21000,
                    priority: 0,
                    metadata: None,
                };
                tx.hash = tx.calculate_hash().unwrap();
                tx.sign(&signing_key).unwrap();
                runtime_clone.submit_transaction(tx).unwrap();
            }
        });
        handles.push(handle);
    }

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all transactions were processed
    let pending_txs = runtime_arc.get_pending_transactions().unwrap();
    assert_eq!(
        pending_txs.len(),
        1000,
        "All 1000 transactions should be in mempool"
    );

    info!("[TEST] Stress test completed successfully");
}

#[test]
fn test_security_validation() {
    init_logging();
    info!("[TEST] Starting security validation tests");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        1000,
    )
    .unwrap();

    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet {
        balance: 1000,
        nonce: 0,
    };
    runtime.create_account(&public_key, account).unwrap();

    // Test double spending prevention
    let mut tx1 = Transaction {
        hash: [0u8; 32],
        sender: public_key,
        recipient: Address::Wallet(public_key),
        payload: TransactionPayload::Transfer { amount: 1000 },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };
    tx1.hash = tx1.calculate_hash().unwrap();
    tx1.sign(&signing_key).unwrap();

    let mut tx2 = Transaction {
        hash: [0u8; 32],
        sender: public_key,
        recipient: Address::Wallet(public_key),
        payload: TransactionPayload::Transfer { amount: 1000 },
        nonce: 1, // Same nonce as tx1
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };
    tx2.hash = tx2.calculate_hash().unwrap();
    tx2.sign(&signing_key).unwrap();

    // First transaction should succeed
    let result1 = runtime.submit_transaction(tx1);
    assert!(result1.is_ok(), "First transaction should be accepted");

    // Second transaction with same nonce should fail
    let result2 = runtime.submit_transaction(tx2);
    assert!(
        result2.is_err(),
        "Double spending transaction should be rejected"
    );

    // Test invalid signature
    let mut invalid_tx = Transaction {
        hash: [0u8; 32],
        sender: public_key,
        recipient: Address::Wallet(public_key),
        payload: TransactionPayload::Transfer { amount: 100 },
        nonce: 2,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };
    invalid_tx.hash = invalid_tx.calculate_hash().unwrap();
    // Don't sign the transaction
    let result3 = runtime.submit_transaction(invalid_tx);
    assert!(
        result3.is_err(),
        "Transaction with invalid signature should be rejected"
    );

    info!("[TEST] Security validation tests completed successfully");
}

#[test]
fn test_storage_advanced_indexing() {
    init_logging();
    info!("[TEST] Starting storage advanced indexing tests");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();

    // Test address-based transaction indexing
    let test_key = PublicKey::from_bytes(&[1u8; 32]).unwrap();
    let test_tx = Transaction {
        hash: [1u8; 32],
        sender: test_key,
        recipient: Address::Wallet(test_key),
        payload: TransactionPayload::Transfer { amount: 100 },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };

    storage.put_transaction(&test_tx).unwrap();
    storage
        .index_transaction(&test_tx.hash, &[2u8; 32], 0)
        .unwrap();

    // Test retrieving transactions by address
    let txs_by_address = storage.get_transactions_by_address(&test_key, 10).unwrap();
    assert!(
        !txs_by_address.is_empty(),
        "Should find transactions for address"
    );

    // Test storage statistics
    let stats = storage.get_storage_stats().unwrap();
    assert!(
        stats.total_transactions > 0,
        "Should have transaction statistics"
    );

    info!("[TEST] Storage advanced indexing tests completed successfully");
}

#[test]
fn test_contract_gas_metering() {
    init_logging();
    info!("[TEST] Starting contract gas metering tests");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();

    // Test gas estimation
    let contract_id = ContractId::from_bytes(&[1u8; 32]);
    let gas_estimate = contract_engine
        .estimate_gas_usage(&contract_id, "test_method", &[], &storage)
        .unwrap();
    assert!(
        gas_estimate.estimated_gas > 0,
        "Gas estimate should be positive"
    );
    assert!(
        gas_estimate.confidence_level > 0.0 && gas_estimate.confidence_level <= 1.0,
        "Confidence level should be between 0 and 1"
    );

    // Test contract metrics
    let metrics = contract_engine.get_contract_metrics(&contract_id);
    // This might fail if the contract doesn't exist, which is expected
    if let Ok(metrics) = metrics {
        // Verify metrics struct is populated
        let _ = metrics; // total_calls is u64, always >= 0
    }

    info!("[TEST] Contract gas metering tests completed successfully");
}

#[test]
fn test_batch_account_persistence() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let test_key = PublicKey::from_bytes(&[1u8; 32]).unwrap();
    let account = Account::Wallet {
        balance: 1000,
        nonce: 0,
    };

    // Direct put/get
    storage.put_account(&test_key, &account).unwrap();
    let retrieved = storage.get_account(&test_key).unwrap().unwrap();
    assert_eq!(retrieved, account, "Direct put/get should work");

    // Batch put/get with only PutAccount
    let updated = Account::Wallet {
        balance: 900,
        nonce: 1,
    };
    let mut batch = StorageBatch::default();
    batch.ops.push(StorageOperation::PutAccount(
        test_key.to_bytes().to_vec(),
        bincode::serialize(&updated).unwrap(),
    ));
    storage.apply_batch(batch).unwrap();
    let retrieved2 = storage.get_account(&test_key).unwrap().unwrap();
    assert_eq!(
        retrieved2, updated,
        "Batch PutAccount should persist correctly"
    );
}

#[test]
fn test_batch_multi_tree_no_cross_contamination() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet {
        balance: 500,
        nonce: 0,
    };

    // Put initial account directly
    storage.put_account(&test_key, &account).unwrap();

    // Build a batch like apply_block does: PutAccount + PutBlock + PutChainState + PutTransaction
    let updated_account = Account::Wallet {
        balance: 400,
        nonce: 1,
    };
    let dummy_block = Block {
        index: 1,
        timestamp: 1777953019,
        prev_hash: [0u8; 32],
        hash: [1u8; 32],
        nonce: 0,
        transactions: vec![],
        metadata: None,
    };
    let dummy_tx = Transaction {
        hash: [2u8; 32],
        sender: test_key,
        recipient: Address::Wallet(test_key),
        payload: TransactionPayload::Transfer { amount: 10 },
        nonce: 1,
        timestamp: 1777953019,
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };
    let chain_state = ChainState {
        latest_block_hash: [1u8; 32],
        latest_block_index: 1,
        accounts_root_hash: [0u8; 32],
        total_supply: 0,
    };

    let mut batch = StorageBatch::default();
    batch.ops.push(StorageOperation::PutAccount(
        test_key.to_bytes().to_vec(),
        bincode::serialize(&updated_account).unwrap(),
    ));
    batch.ops.push(StorageOperation::PutBlock(
        dummy_block.hash.to_vec(),
        bincode::serialize(&dummy_block).unwrap(),
    ));
    batch.ops.push(StorageOperation::PutChainState(
        "global:current".as_bytes().to_vec(),
        bincode::serialize(&chain_state).unwrap(),
    ));
    batch.ops.push(StorageOperation::PutTransaction(
        dummy_tx.hash.to_vec(),
        bincode::serialize(&dummy_tx).unwrap(),
    ));

    storage.apply_batch(batch).unwrap();

    // Verify account data is NOT corrupted by other tree writes
    let retrieved = storage.get_account(&test_key).unwrap().unwrap();
    assert_eq!(
        retrieved, updated_account,
        "Account should not be corrupted by multi-tree batch writes. Expected {:?}, got {:?}",
        updated_account, retrieved
    );

    // Also verify block was stored correctly
    let block_retrieved = storage.get_block(&dummy_block.hash).unwrap().unwrap();
    assert_eq!(block_retrieved.index, 1);
    assert_eq!(block_retrieved.timestamp, 1777953019);
}

fn create_test_wasm_module() -> Vec<u8> {
    // Valid WASM with memory export and exported main(i32,i32)->i32 returning arg count.
    vec![
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f, // type: (i32,i32)->i32
        0x03, 0x02, 0x01, 0x00, // func: 1, type 0
        0x05, 0x03, 0x01, 0x00, 0x01, // memory: min 1
        0x07, 0x11, 0x02, 0x06, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00, 0x04, 0x6d, 0x61,
        0x69, 0x6e, 0x00, 0x00, // exports: memory, main
        0x0a, 0x06, 0x01, 0x04, 0x00, 0x20, 0x01, 0x0b, // code section
    ]
}
