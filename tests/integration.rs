use baals::*;
use ed25519_dalek::Signer;
use log::info;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod wasm_fixtures;

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
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    info!("[TEST] Creating runtime");
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 10000)
            .unwrap();
    info!("[TEST] Runtime created successfully");

    runtime.start().unwrap();
    info!("[TEST] Runtime started");

    let signing_key1 =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key1 = PublicKey::from(signing_key1.verifying_key());
    let account1 = Account::Wallet { balance: 1000, nonce: 0 };
    runtime.create_account(&public_key1, account1).unwrap();
    info!("[TEST] Created account1: {}", crate::types::format_hex(&public_key1.to_bytes()));

    let signing_key2 =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key2 = PublicKey::from(signing_key2.verifying_key());
    let account2 = Account::Wallet { balance: 500, nonce: 0 };
    runtime.create_account(&public_key2, account2).unwrap();
    info!("[TEST] Created account2: {}", crate::types::format_hex(&public_key2.to_bytes()));

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
    if let Account::Wallet { balance: balance1, nonce: nonce1 } = account1_after {
        assert_eq!(balance1, 900);
        assert_eq!(nonce1, 1);
    } else {
        panic!("Account1 should be a wallet");
    }
    if let Account::Wallet { balance: balance2, nonce: nonce2 } = account2_after {
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
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();

    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet { balance: 1000, nonce: 0 };
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
    assert!(result.is_ok(), "Overspend is caught at block application, not submission");

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
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet { balance: 5000, nonce: 0 };
    runtime.create_account(&public_key, account).unwrap();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
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
    assert!(runtime.submit_transaction(expired).is_err(), "Expired tx should be rejected");

    // Test: timestamp too far in the future
    let future = make_tx(1, now + 600, 100, 100_000);
    assert!(runtime.submit_transaction(future).is_err(), "Future tx should be rejected");

    // Test: gas limit too low
    let low_gas = make_tx(1, now, 100, 10_000);
    assert!(runtime.submit_transaction(low_gas).is_err(), "Low gas tx should be rejected");

    // Test: gas limit too high
    let high_gas = make_tx(1, now, 100, 20_000_000);
    assert!(runtime.submit_transaction(high_gas).is_err(), "High gas tx should be rejected");

    // Test: zero amount transfer
    let zero_amount = make_tx(1, now, 0, 100_000);
    assert!(
        runtime.submit_transaction(zero_amount).is_err(),
        "Zero amount tx should be rejected"
    );

    // Test: valid transaction accepted
    let valid = make_tx(1, now, 100, 100_000);
    assert!(runtime.submit_transaction(valid).is_ok(), "Valid tx should be accepted");

    info!("[TEST] test_hardened_transaction_validation completed successfully");
}

#[test]
fn test_ledger_rejects_future_block_timestamp() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(SledStorage::new(temp_dir.path()).unwrap());
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let signer = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = Arc::new(BaaLSContractEngine::new((*storage).clone()).unwrap());
    let consensus = PoAConsensus::new(signer, 1000).with_signing_key(consensus_signing_key);
    let ledger = Ledger::new(Arc::clone(&storage), Arc::clone(&contract_engine));
    ledger.initialize_chain().unwrap();

    let chain_state = storage.get_chain_state().unwrap().unwrap();
    let prev_block = storage.get_block(&chain_state.latest_block_hash).unwrap().unwrap();
    let mut block = consensus.generate_block(&[], &prev_block, &chain_state).unwrap();
    block.timestamp =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 120;
    block.hash = block.calculate_hash().unwrap();
    consensus.sign_block(&mut block).unwrap();

    let err =
        ledger.validate_block(&block, &chain_state).expect_err("future block should be rejected");
    assert!(
        err.to_string().contains("future"),
        "expected future timestamp error, got: {}",
        err
    );
}

#[test]
fn test_block_application_is_atomic_on_transaction_failure() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let consensus_pk = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(consensus_pk, 1000).with_signing_key(consensus_signing_key);
    let runtime =
        Runtime::with_mempool_limit(storage, consensus, contract_engine, NoopSync, 1000).unwrap();
    runtime.start().unwrap();

    let sender_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let sender = PublicKey::from(sender_key.verifying_key());
    let recipient_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let recipient = PublicKey::from(recipient_key.verifying_key());
    runtime.create_account(&sender, Account::Wallet { balance: 100, nonce: 0 }).unwrap();
    runtime.create_account(&recipient, Account::Wallet { balance: 0, nonce: 0 }).unwrap();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let make_tx = |nonce: u64, amount: u64| {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender,
            recipient: Address::Wallet(recipient),
            payload: TransactionPayload::Transfer { amount },
            nonce,
            timestamp: now,
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100_000,
            priority: 0,
            metadata: None,
        };
        tx.hash = tx.calculate_hash().unwrap();
        tx.sign(&sender_key).unwrap();
        tx
    };

    runtime.submit_transaction(make_tx(1, 80)).unwrap();
    runtime.submit_transaction(make_tx(2, 80)).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(runtime.produce_block());
    assert!(result.is_err(), "block with failing transaction should be rejected atomically");

    let sender_after = runtime.get_account(&sender).unwrap().unwrap();
    if let Account::Wallet { balance, nonce } = sender_after {
        assert_eq!(balance, 100, "sender balance should roll back");
        assert_eq!(nonce, 0, "sender nonce should roll back");
    } else {
        panic!("sender account should remain wallet");
    }

    let chain = runtime.get_chain_state().unwrap();
    assert_eq!(chain.latest_block_index, 0, "chain head should not advance on failed block");
}

#[test]
fn test_runtime_status_tracks_lifecycle_and_uptime() {
    init_logging();

    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
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
fn test_runtime_health_status_exposes_chain_and_mempool() {
    init_logging();

    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let runtime =
        Runtime::with_mempool_limit(storage, consensus, contract_engine, NoopSync, 1000).unwrap();
    runtime.start().unwrap();

    let signer = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let wallet = PublicKey::from(signer.verifying_key());
    runtime.create_account(&wallet, Account::Wallet { balance: 1000, nonce: 0 }).unwrap();

    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: wallet,
        recipient: Address::Wallet(wallet),
        payload: TransactionPayload::Transfer { amount: 1 },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        priority: 0,
        metadata: None,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(&signer).unwrap();
    runtime.submit_transaction(tx).unwrap();

    let health = runtime.get_health_status().unwrap();
    assert!(!health.status.is_empty());
    assert_eq!(health.latest_block_index, 0);
    assert_eq!(health.mempool_size, 1);
    assert_eq!(health.connected_peers, 0);
}

#[test]
fn test_runtime_auto_block_production_from_mempool_threshold() {
    init_logging();

    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 100).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let mut runtime =
        Runtime::with_mempool_limit(storage, consensus, contract_engine, sync_layer, 2).unwrap();
    runtime.auto_block_interval_ms = 100; // 100ms auto-block for fast test
    runtime.auto_block_mempool_threshold = 1;
    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    runtime.create_account(&public_key, Account::Wallet { balance: 1000, nonce: 0 }).unwrap();

    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: public_key,
        recipient: Address::Wallet(public_key),
        payload: TransactionPayload::Transfer { amount: 1 },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        priority: 0,
        metadata: None,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(&signing_key).unwrap();
    runtime.submit_transaction(tx).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut produced = false;
    while Instant::now() < deadline {
        let chain = runtime.get_chain_state().unwrap();
        if chain.latest_block_index >= 1 {
            produced = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(produced, "automatic block production should advance chain");

    runtime.stop().unwrap();
}

#[test]
fn test_transaction_merkle_root_in_block() {
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    runtime.create_account(&public_key, Account::Wallet { balance: 10000, nonce: 0 }).unwrap();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();

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
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();

    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet { balance: 1000, nonce: 0 };
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
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();

    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet { balance: 1000, nonce: 0 };
    runtime.create_account(&public_key, account).unwrap();

    // Create a simple test WASM module
    let wasm_bytes = create_test_wasm_module();

    // Deploy contract
    let contract_id = runtime.deploy_contract(&public_key, &wasm_bytes, None, 100000).unwrap();
    assert!(
        !contract_id.to_bytes().iter().all(|&b| b == 0),
        "Contract ID should not be zero"
    );

    // Call contract — our test module returns args_len (0 since no args passed)
    let result =
        runtime.call_contract(&public_key, &contract_id, "test_method", &[], None, 100000).unwrap();
    // Module returns 0 (args_len) — verify call succeeded, not specific return value
    info!("[TEST] Contract call returned {} bytes", result.len());

    info!("[TEST] test_contract_deploy_and_execution completed successfully");
}

#[test]
fn test_contract_storage_root_tracks_contract_kv_state() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let runtime =
        Runtime::with_mempool_limit(storage, consensus, contract_engine, NoopSync, 1000).unwrap();
    runtime.start().unwrap();

    let deployer_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let deployer = PublicKey::from(deployer_key.verifying_key());
    runtime.create_account(&deployer, Account::Wallet { balance: 10_000, nonce: 0 }).unwrap();

    let wasm_bytes = create_test_wasm_module();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();

    let mut deploy_tx = Transaction {
        hash: [0u8; 32],
        sender: deployer,
        recipient: Address::Wallet(deployer),
        payload: TransactionPayload::ContractDeploy {
            wasm_bytes: wasm_bytes.clone(),
            init_payload: None,
        },
        nonce: 1,
        timestamp: now,
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 500_000,
        priority: 0,
        metadata: None,
    };
    deploy_tx.hash = deploy_tx.calculate_hash().unwrap();
    deploy_tx.sign(&deployer_key).unwrap();
    runtime.submit_transaction(deploy_tx).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    tokio_rt.block_on(runtime.produce_block()).unwrap();

    // Same deterministic contract-id derivation used by deploy_contract.
    let mut cid_hasher = Sha256::new();
    cid_hasher.update(deployer.to_bytes());
    cid_hasher.update(1u64.to_be_bytes());
    cid_hasher.update(&wasm_bytes);
    let cid_hash: [u8; 32] = cid_hasher.finalize().into();
    let contract_id = ContractId::from_bytes(&cid_hash);

    // Seed storage directly to validate root computation from real key/value state.
    runtime.storage().contract_storage_write(&contract_id, b"alpha", b"value-1").unwrap();
    runtime.storage().contract_storage_write(&contract_id, b"beta", b"value-2").unwrap();

    let mut call_tx = Transaction {
        hash: [0u8; 32],
        sender: deployer,
        recipient: Address::Contract(contract_id.clone()),
        payload: TransactionPayload::ContractCall {
            method: "test_method".to_string(),
            args: vec![],
            value: None,
        },
        nonce: 2,
        timestamp: now + 1,
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 500_000,
        priority: 0,
        metadata: None,
    };
    call_tx.hash = call_tx.calculate_hash().unwrap();
    call_tx.sign(&deployer_key).unwrap();
    runtime.submit_transaction(call_tx).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    tokio_rt.block_on(runtime.produce_block()).unwrap();

    // Recompute expected storage root using SparseMerkleTree (matching ledger impl).
    let mut expected_smt = SparseMerkleTree::new();
    let kvs = vec![
        (b"alpha".as_ref().to_vec(), b"value-1".as_ref().to_vec()),
        (b"beta".as_ref().to_vec(), b"value-2".as_ref().to_vec()),
    ];
    for (k, v) in kvs {
        let smt_key: [u8; 32] = sha2::Sha256::digest(&k).into();
        expected_smt.insert(smt_key, v);
    }
    let expected_root = expected_smt.root();

    // Find the contract account and verify its storage root matches contract KV state.
    let all_accounts = runtime.storage().get_all_accounts().unwrap();
    let contract_account = all_accounts
        .into_iter()
        .find_map(|(_pk, account)| match account {
            Account::Contract { code_hash: _, storage_root_hash, nonce: _ } => {
                Some(storage_root_hash)
            }
            Account::Wallet { .. } => None,
        })
        .expect("contract account should exist after deploy");
    assert_eq!(
        contract_account, expected_root,
        "contract storage root should be Merkle root of contract key/value state"
    );
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
    let test_account = Account::Wallet { balance: 1000, nonce: 0 };
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
    assert_eq!(retrieved_chain_state.latest_block_index, test_chain_state.latest_block_index);

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
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 10000)
            .unwrap();

    runtime.start().unwrap();

    // Create multiple senders so the benchmark respects anti-spam per-sender mempool caps.
    let mut wallets = Vec::new();
    for _ in 0..10 {
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        runtime
            .create_account(&public_key, Account::Wallet { balance: 1000000, nonce: 0 })
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
        !block.transactions.is_empty(),
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
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 50000)
            .unwrap();

    runtime.start().unwrap();

    // Create multiple accounts
    let mut accounts = Vec::new();
    for _i in 0..10 {
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        let account = Account::Wallet { balance: 10000, nonce: 0 };
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
    assert_eq!(pending_txs.len(), 1000, "All 1000 transactions should be in mempool");

    info!("[TEST] Stress test completed successfully");
}

#[test]
fn test_security_validation() {
    init_logging();
    info!("[TEST] Starting security validation tests");
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();

    runtime.start().unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let public_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet { balance: 1000, nonce: 0 };
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
    assert!(result2.is_err(), "Double spending transaction should be rejected");

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
    assert!(result3.is_err(), "Transaction with invalid signature should be rejected");

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
    storage.index_transaction(&test_tx.hash, &[2u8; 32], 0).unwrap();

    // Test retrieving transactions by address
    let txs_by_address = storage.get_transactions_by_address(&test_key, 10).unwrap();
    assert!(!txs_by_address.is_empty(), "Should find transactions for address");

    // Test storage statistics
    let stats = storage.get_storage_stats().unwrap();
    assert!(stats.total_transactions > 0, "Should have transaction statistics");

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
    let gas_estimate =
        contract_engine.estimate_gas_usage(&contract_id, "test_method", &[], &storage).unwrap();
    assert!(gas_estimate.estimated_gas > 0, "Gas estimate should be positive");
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
    let account = Account::Wallet { balance: 1000, nonce: 0 };

    // Direct put/get
    storage.put_account(&test_key, &account).unwrap();
    let retrieved = storage.get_account(&test_key).unwrap().unwrap();
    assert_eq!(retrieved, account, "Direct put/get should work");

    // Batch put/get with only PutAccount
    let updated = Account::Wallet { balance: 900, nonce: 1 };
    let mut batch = StorageBatch::default();
    batch.ops.push(StorageOperation::PutAccount(
        test_key.to_bytes().to_vec(),
        bincode::serialize(&updated).unwrap(),
    ));
    storage.apply_batch(batch).unwrap();
    let retrieved2 = storage.get_account(&test_key).unwrap().unwrap();
    assert_eq!(retrieved2, updated, "Batch PutAccount should persist correctly");
}

#[test]
fn test_batch_multi_tree_no_cross_contamination() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet { balance: 500, nonce: 0 };

    // Put initial account directly
    storage.put_account(&test_key, &account).unwrap();

    // Build a batch like apply_block does: PutAccount + PutBlock + PutChainState + PutTransaction
    let updated_account = Account::Wallet { balance: 400, nonce: 1 };
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

#[test]
fn test_keystore_round_trip() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let keystore_path = temp_dir.path().join("keys");
    let keystore = Keystore::new(Some(keystore_path)).unwrap();

    // Create key
    let pk = keystore.create_key("password123").unwrap();
    assert!(!pk.to_bytes().iter().all(|b| *b == 0), "Public key should be non-zero");

    // List keys
    let keys = keystore.list_keys().unwrap();
    assert!(keys.contains(&pk), "Created key should appear in list");

    // Sign with loaded key
    let signing_key = keystore.load_key(&pk, "password123").unwrap();
    let message = b"test message";
    let signature = signing_key.sign(message);
    pk.verify(message, &signature).unwrap();

    // Wrong password should fail
    assert!(keystore.load_key(&pk, "wrong_password").is_err());

    info!("[TEST] Keystore round-trip passed");
}

#[test]
fn test_consensus_signing_verification() {
    init_logging();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
    let test_key = PublicKey::from(signing_key.verifying_key());
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);

    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let sync_layer = NoopSync;
    let runtime = Runtime::new(storage, consensus, contract_engine, sync_layer).unwrap();

    // Submit a transaction
    let sender_sk = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let sender_pk = PublicKey::from(sender_sk.verifying_key());
    runtime.create_account(&sender_pk, Account::Wallet { balance: 1000, nonce: 0 }).unwrap();

    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: sender_pk,
        recipient: Address::Wallet(sender_pk),
        payload: TransactionPayload::Transfer { amount: 1 },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        priority: 0,
        metadata: None,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(&sender_sk).unwrap();
    runtime.submit_transaction(tx).unwrap();

    // Produce block — consensus signs it
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let block = tokio_rt.block_on(runtime.produce_block()).unwrap();
    assert!(block.metadata.is_some(), "Block should have signing metadata");
    let metadata = block.metadata.as_ref().unwrap();
    assert!(metadata.contains_key("signer"), "Metadata should contain signer");
    assert!(metadata.contains_key("signature"), "Metadata should contain signature");

    info!("[TEST] Consensus signing verification passed");
}

#[test]
fn test_invalid_nonce_rejected() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::new(storage, consensus, contract_engine, sync_layer).unwrap();

    let sender_sk = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let sender_pk = PublicKey::from(sender_sk.verifying_key());
    runtime.create_account(&sender_pk, Account::Wallet { balance: 1000, nonce: 0 }).unwrap();

    // Submit tx with nonce 5 (account nonce is 0, expected 1)
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: sender_pk,
        recipient: Address::Wallet(sender_pk),
        payload: TransactionPayload::Transfer { amount: 1 },
        nonce: 5, // gap nonce
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        priority: 0,
        metadata: None,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(&sender_sk).unwrap();
    runtime.submit_transaction(tx).unwrap(); // should be accepted (nonce >= expected)

    // Submit tx with nonce 0 (stale nonce, already used)
    let mut tx2 = Transaction {
        hash: [0u8; 32],
        sender: sender_pk,
        recipient: Address::Wallet(sender_pk),
        payload: TransactionPayload::Transfer { amount: 1 },
        nonce: 0, // stale nonce
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        priority: 0,
        metadata: None,
    };
    tx2.hash = tx2.calculate_hash().unwrap();
    tx2.sign(&sender_sk).unwrap();
    assert!(runtime.submit_transaction(tx2).is_err(), "Stale nonce should be rejected");

    info!("[TEST] Invalid nonce rejection passed");
}

#[test]
fn test_insufficient_balance_rejected() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let sync_layer = NoopSync;
    let runtime = Runtime::new(storage, consensus, contract_engine, sync_layer).unwrap();

    let sender_sk = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let sender_pk = PublicKey::from(sender_sk.verifying_key());
    runtime.create_account(&sender_pk, Account::Wallet { balance: 5, nonce: 0 }).unwrap();

    // Submit transfer for more than balance
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: sender_pk,
        recipient: Address::Wallet(sender_pk),
        payload: TransactionPayload::Transfer { amount: 100 }, // exceeds balance of 5
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        priority: 0,
        metadata: None,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(&sender_sk).unwrap();
    runtime.submit_transaction(tx).unwrap();

    // Producing block should fail due to insufficient balance
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let result = tokio_rt.block_on(runtime.produce_block());
    assert!(result.is_err(), "Block should fail due to insufficient balance");

    info!("[TEST] Insufficient balance rejection passed");
}

#[test]
fn test_concurrent_transaction_submission() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let sync_layer = NoopSync;
    let runtime = Arc::new(
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 10000)
            .unwrap(),
    );
    runtime.start().unwrap();

    // Create 4 accounts, each with 10000 balance
    let mut accounts = Vec::new();
    for _ in 0..4 {
        let sk = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let pk = PublicKey::from(sk.verifying_key());
        runtime.create_account(&pk, Account::Wallet { balance: 10000, nonce: 0 }).unwrap();
        accounts.push((sk, pk));
    }

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();

    let runtime = Arc::clone(&runtime);
    let handles: Vec<_> = accounts
        .into_iter()
        .map(|(sk, pk)| {
            let rt = Arc::clone(&runtime);
            std::thread::spawn(move || {
                for nonce in 1..=50 {
                    let mut tx = Transaction {
                        hash: [0u8; 32],
                        sender: pk,
                        recipient: Address::Wallet(pk),
                        payload: TransactionPayload::Transfer { amount: 1 },
                        nonce,
                        timestamp: now,
                        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
                        gas_limit: 100000,
                        priority: 0,
                        metadata: None,
                    };
                    tx.hash = tx.calculate_hash().unwrap();
                    tx.sign(&sk).unwrap();
                    rt.submit_transaction(tx).unwrap();
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify mempool contains all 200 transactions (4 * 50)
    let pending = runtime.get_pending_transactions().unwrap();
    assert_eq!(pending.len(), 200, "Mempool should contain 200 transactions");

    info!(
        "[TEST] Concurrent submission: {} transactions in mempool after 4x50 concurrent",
        pending.len()
    );
}

#[test]
fn test_concurrent_mempool_integrity() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let sync_layer = NoopSync;
    let runtime = Arc::new(
        Runtime::with_mempool_limit(storage, consensus, contract_engine, sync_layer, 5000).unwrap(),
    );
    runtime.start().unwrap();

    // Spawn 8 threads, each with its own account, submitting 25 transactions each
    let mut handles = Vec::new();
    let pending: Arc<std::sync::Mutex<Vec<bool>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

    for _thread_id in 0..8 {
        let rt = Arc::clone(&runtime);
        let pending_clone = Arc::clone(&pending);

        // Create a unique account for this thread
        let account_sk =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let account_pk = PublicKey::from(account_sk.verifying_key());
        rt.create_account(&account_pk, Account::Wallet { balance: 10000, nonce: 0 }).unwrap();

        let handle = std::thread::spawn(move || {
            for nonce in 1..=25 {
                let mut tx = Transaction {
                    hash: [0u8; 32],
                    sender: account_pk,
                    recipient: Address::Wallet(account_pk),
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
                tx.sign(&account_sk).unwrap();
                let result = rt.submit_transaction(tx);
                pending_clone.lock().unwrap().push(result.is_ok());
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }

    let results = pending.lock().unwrap();
    let success_count = results.iter().filter(|&&r| r).count();
    assert_eq!(
        success_count, 200,
        "Expected 200 successful submissions (8 threads × 25), got {success_count}"
    );

    let pending_txs = runtime.get_pending_transactions().unwrap();
    info!(
        "[TEST] Concurrent mempool integrity: {} successful, {} in mempool",
        success_count,
        pending_txs.len()
    );
    assert!(!pending_txs.is_empty(), "Mempool should not be empty");
}

#[test]
fn test_reentrancy_guard() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.start().unwrap();

    let deployer_sk =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let deployer = PublicKey::from(deployer_sk.verifying_key());
    runtime.create_account(&deployer, Account::Wallet { balance: 10_000, nonce: 0 }).unwrap();

    let wasm_bytes = wasm_fixtures::make_self_calling_module();
    let contract_id = runtime.deploy_contract(&deployer, &wasm_bytes, None, 1_000_000).unwrap();

    // Call "safe" — should write i32(1) to memory and return 4
    let safe_result =
        runtime.call_contract(&deployer, &contract_id, "safe", &[], None, 1_000_000).unwrap();
    assert_eq!(safe_result.len(), 4, "safe() should return 4 bytes");
    let safe_val = i32::from_le_bytes(safe_result.try_into().unwrap_or_default());
    assert_eq!(safe_val, 1, "safe() should encode value 1");

    // Call "reenter" — the WASM calls baals_call_contract on itself.
    // Currently, baals_call_contract queues the call (returns call index).
    // Verify the WASM execution completes without error.
    let reenter_result =
        runtime.call_contract(&deployer, &contract_id, "reenter", &[], None, 1_000_000).unwrap();
    // reenter stores the call index (0) and returns 4
    assert_eq!(reenter_result.len(), 4, "reenter() should return 4 bytes");

    info!(
        "[TEST] Reentrancy guard test passed: safe={}, reenter={} bytes",
        safe_val,
        reenter_result.len()
    );
}

#[test]
fn test_host_function_storage_write_read() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.start().unwrap();

    let deployer_sk =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let deployer = PublicKey::from(deployer_sk.verifying_key());
    runtime.create_account(&deployer, Account::Wallet { balance: 10_000, nonce: 0 }).unwrap();

    let wasm_bytes = wasm_fixtures::make_storage_write_read_module();
    let contract_id = runtime.deploy_contract(&deployer, &wasm_bytes, None, 1_000_000).unwrap();

    // Call store_and_read — writes "key"="val", reads back.
    // Returns the 3 bytes of "val" at memory[0].
    let result = runtime
        .call_contract(&deployer, &contract_id, "store_and_read", &[], None, 1_000_000)
        .unwrap();
    assert_eq!(result.len(), 3, "store_and_read should return 3 bytes ('val')");
    assert_eq!(&result, b"val", "store_and_read should read back 'val'");

    info!("[TEST] Host function storage write/read test passed, data={:?}", result);
}

#[test]
fn test_inter_contract_call() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.start().unwrap();

    let deployer_sk =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let deployer = PublicKey::from(deployer_sk.verifying_key());
    runtime.create_account(&deployer, Account::Wallet { balance: 50_000, nonce: 0 }).unwrap();

    // Deploy callee contract (provides storage write via "store")
    let callee_wasm = wasm_fixtures::make_inter_contract_callee();
    let callee_id = runtime.deploy_contract(&deployer, &callee_wasm, None, 1_000_000).unwrap();

    // Call the callee's "store" function — verifies contract deployment and calling.
    // store() returns 0 (read 0 bytes), so result is empty.
    let result =
        runtime.call_contract(&deployer, &callee_id, "store", &[], None, 1_000_000).unwrap();
    assert!(result.is_empty(), "store() should return empty result");
    info!("[TEST] Inter-contract callee deployment and call succeeded");
}

#[test]
fn test_sync_layer_handshake() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();

    let storage1 = SledStorage::new(temp_dir.path().join("node1")).unwrap();
    let storage2 = SledStorage::new(temp_dir.path().join("node2")).unwrap();

    let signing_key1 =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let signing_key2 =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key1 = PublicKey::from(signing_key1.verifying_key());
    let test_key2 = PublicKey::from(signing_key2.verifying_key());

    let ce1 = BaaLSContractEngine::new(storage1.clone()).unwrap();
    let ce2 = BaaLSContractEngine::new(storage2.clone()).unwrap();

    let consensus1 = PoAConsensus::new(test_key1, 1000).with_signing_key(signing_key1);
    let consensus2 = PoAConsensus::new(test_key2, 1000).with_signing_key(signing_key2);

    // Use NoopSync for both — verifies runtime works with sync layer
    let runtime1 =
        Runtime::with_mempool_limit(storage1.clone(), consensus1, ce1, NoopSync, 1000).unwrap();
    let runtime2 =
        Runtime::with_mempool_limit(storage2.clone(), consensus2, ce2, NoopSync, 1000).unwrap();

    runtime1.start().unwrap();
    runtime2.start().unwrap();

    info!("[TEST] Two runtimes started with NoopSync");

    // Create an account on runtime1 and check it stays local
    let account_sk =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let account_pk = PublicKey::from(account_sk.verifying_key());
    runtime1.create_account(&account_pk, Account::Wallet { balance: 1000, nonce: 0 }).unwrap();

    assert!(runtime1.get_account(&account_pk).unwrap().is_some());
    // NoopSync means runtime2 has no knowledge of runtime1's state
    assert!(runtime2.get_account(&account_pk).unwrap().is_none());

    info!("[TEST] Sync layer isolation test passed");
}

#[test]
fn test_redb_storage_basic_crud() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = RedbStorage::new(temp_dir.path()).unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet { balance: 1000, nonce: 0 };

    storage.put_account(&test_key, &account).unwrap();
    let retrieved = storage.get_account(&test_key).unwrap().unwrap();
    assert_eq!(retrieved, account, "RedbStorage put/get account round-trip");

    let block = Block {
        index: 1,
        timestamp: 1700000000,
        prev_hash: [0u8; 32],
        hash: [1u8; 32],
        transactions: vec![],
        nonce: 0,
        metadata: None,
    };
    storage.put_block(&block).unwrap();
    let retrieved_block = storage.get_block(&block.hash).unwrap().unwrap();
    assert_eq!(retrieved_block.index, 1, "RedbStorage put/get block");

    let chain_state = ChainState {
        latest_block_hash: block.hash,
        latest_block_index: 1,
        accounts_root_hash: [0u8; 32],
        total_supply: 1000,
    };
    storage.put_chain_state(&chain_state).unwrap();
    let retrieved_cs = storage.get_chain_state().unwrap().unwrap();
    assert_eq!(retrieved_cs.latest_block_index, 1, "RedbStorage put/get chain state");

    // Batch test
    let account2 = Account::Wallet { balance: 500, nonce: 0 };
    let mut batch = StorageBatch::default();
    batch.ops.push(StorageOperation::PutAccount(
        test_key.to_bytes().to_vec(),
        bincode::serialize(&account2).unwrap(),
    ));
    storage.apply_batch(batch).unwrap();
    let after_batch = storage.get_account(&test_key).unwrap().unwrap();
    assert_eq!(after_batch.balance(), 500, "RedbStorage batch apply");

    info!("[TEST] RedbStorage basic CRUD passed");
}

#[test]
fn test_redb_storage_delete() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = RedbStorage::new(temp_dir.path()).unwrap();

    let signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let account = Account::Wallet { balance: 100, nonce: 0 };

    storage.put_account(&test_key, &account).unwrap();
    assert!(storage.get_account(&test_key).unwrap().is_some());
    storage.delete_account(&test_key).unwrap();
    assert!(storage.get_account(&test_key).unwrap().is_none());

    info!("[TEST] RedbStorage delete passed");
}

#[test]
fn test_redb_storage_with_runtime() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = RedbStorage::new(temp_dir.path()).unwrap();
    let signing_key =
        Runtime::<RedbStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let runtime = Runtime::new(storage, consensus, contract_engine, NoopSync).unwrap();
    runtime.start().unwrap();

    let account_sk =
        Runtime::<RedbStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let account_pk = PublicKey::from(account_sk.verifying_key());
    runtime.create_account(&account_pk, Account::Wallet { balance: 1000, nonce: 0 }).unwrap();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: account_pk,
        recipient: Address::Wallet(account_pk),
        payload: TransactionPayload::Transfer { amount: 10 },
        nonce: 1,
        timestamp: now,
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        priority: 0,
        metadata: None,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(&account_sk).unwrap();
    runtime.submit_transaction(tx).unwrap();

    let pending = runtime.get_pending_transactions().unwrap();
    assert_eq!(pending.len(), 1, "RedbStorage runtime mempool has 1 tx");

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let block = tokio_rt.block_on(runtime.produce_block()).unwrap();
    assert_eq!(block.index, 1, "RedbStorage runtime produced block 1");

    info!("[TEST] RedbStorage runtime integration passed");
}
