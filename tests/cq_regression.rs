use baals::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

mod wasm_fixtures;

fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .try_init();
}

fn make_runtime(
    data_dir: &std::path::Path,
) -> (
    Runtime<SledStorage, PoAConsensus, NoopSync>,
    ed25519_dalek::SigningKey,
    PublicKey,
) {
    let storage = SledStorage::new(data_dir).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus =
        PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key.clone());
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.start().unwrap();
    (runtime, consensus_signing_key, test_key)
}

fn make_test_account(
    runtime: &Runtime<SledStorage, PoAConsensus, NoopSync>,
    balance: u64,
) -> (ed25519_dalek::SigningKey, PublicKey) {
    let sk = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let pk = PublicKey::from(sk.verifying_key());
    runtime.create_account(&pk, Account::Wallet { balance, nonce: 0 }).unwrap();
    (sk, pk)
}

#[allow(clippy::too_many_arguments)]
fn make_transfer_tx(
    sender_pk: PublicKey,
    sender_sk: &ed25519_dalek::SigningKey,
    recipient: PublicKey,
    amount: u64,
    nonce: u64,
    gas_limit: u64,
    gas_price: u64,
    chain_id: u64,
) -> Transaction {
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: sender_pk,
        recipient: Address::Wallet(recipient),
        payload: TransactionPayload::Transfer { amount },
        nonce,
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit,
        gas_price,
        priority: 0,
        metadata: None,
        chain_id,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(sender_sk).unwrap();
    tx
}

#[allow(clippy::too_many_arguments)]
fn make_contract_call_tx(
    sender_pk: PublicKey,
    sender_sk: &ed25519_dalek::SigningKey,
    contract_id: ContractId,
    method: &str,
    args: Vec<Vec<u8>>,
    value: Option<u64>,
    nonce: u64,
    gas_limit: u64,
    gas_price: u64,
    chain_id: u64,
) -> Transaction {
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: sender_pk,
        recipient: Address::Contract(contract_id),
        payload: TransactionPayload::ContractCall { method: method.to_string(), args, value },
        nonce,
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit,
        gas_price,
        priority: 0,
        metadata: None,
        chain_id,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(sender_sk).unwrap();
    tx
}

// CQ-3: Balance check before mempool acceptance
#[test]
fn cq3_balance_check_rejects_insufficient_funds() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (sk, pk) = make_test_account(&runtime, 10_000);

    // This tx costs 21_000 gas + 100 value = 21_100, which exceeds balance
    let tx = make_transfer_tx(pk, &sk, pk, 100, 1, 21_000, 1, 1);
    let result = runtime.submit_transaction(tx);
    assert!(
        result.is_err(),
        "CQ-3: submit_transaction should reject tx with insufficient balance"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Insufficient balance"),
        "Expected Insufficient balance error, got: {}",
        err_msg
    );
}

// CQ-4: chain_id validation prevents cross-chain replay
#[test]
fn cq4_invalid_chain_id_rejected() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (sk, pk) = make_test_account(&runtime, 1_000_000);

    // tx with wrong chain_id should be rejected
    let tx = make_transfer_tx(pk, &sk, pk, 100, 1, 21_000, 1, 999);
    let result = runtime.submit_transaction(tx);
    assert!(result.is_err(), "CQ-4: submit_transaction should reject tx with wrong chain_id");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Invalid chain_id"),
        "Expected Invalid chain_id error, got: {}",
        err_msg
    );
}

// CQ-6 / CQ-16: gas_price=0 rejected, min gas price enforced
#[test]
fn cq6_gas_price_zero_rejected() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (sk, pk) = make_test_account(&runtime, 1_000_000);

    let tx = make_transfer_tx(pk, &sk, pk, 100, 1, 21_000, 0, 1);
    let result = runtime.submit_transaction(tx);
    assert!(
        result.is_err(),
        "CQ-6/CQ-16: submit_transaction should reject tx with gas_price=0"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Gas price too low"),
        "Expected Gas price too low error, got: {}",
        err_msg
    );
}

// CQ-8: Monotonic block timestamps enforced
#[test]
fn cq8_monotonic_timestamp_enforced() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_sk =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_sk.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_sk);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.start().unwrap();

    let (sk, pk) = make_test_account(&runtime, 1_000_000);
    let tx = make_transfer_tx(pk, &sk, pk, 100, 1, 21_000, 1, 1);
    runtime.submit_transaction(tx).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let block1 = tokio_rt.block_on(runtime.produce_block()).unwrap();

    // Create block2 with timestamp BEFORE block1's timestamp
    let mut block2 = Block {
        index: block1.index + 1,
        timestamp: block1.timestamp.saturating_sub(1),
        prev_hash: block1.hash,
        state_root: [0u8; 32],
        hash: [0u8; 32],
        nonce: 0,
        transactions: vec![],
        metadata: None,
                total_gas_used: 0,
                signer: None,
                signature: None,
                quorum_signatures: Vec::new(),
            };
    block2.hash = block2.calculate_hash().unwrap();

    let result = runtime.ledger().validate_block(&block2);
    assert!(
        result.is_err(),
        "CQ-8: validate_block should reject block with timestamp < parent timestamp"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("timestamp") || err_msg.contains("Timestamp"),
        "Expected timestamp error, got: {}",
        err_msg
    );
}

// CQ-11: Failed transaction status recorded in storage
// Tests the ledger path directly by constructing a block with a tx that will fail
// inside apply_block (transfer amount exceeds balance after gas).
#[test]
fn cq11_failed_tx_status_recorded() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, consensus_sk, consensus_pk) = make_runtime(temp_dir.path());

    // Create sender with balance less than transfer amount
    let (sk, pk) = make_test_account(&runtime, 1_000);

    // Create tx that transfers more than balance â€” this would be rejected by mempool (CQ-3),
    // so we bypass mempool and feed the block directly to the ledger.
    let tx = make_transfer_tx(pk, &sk, pk, 2_000, 1, 21_000, 1, 1);

    let prev_block = runtime.get_block_by_height(0).unwrap().unwrap();
    let mut block = Block {
        index: 1,
        timestamp: prev_block.timestamp + 1,
        prev_hash: prev_block.hash,
        state_root: [0u8; 32],
        hash: [0u8; 32],
        nonce: 0,
        transactions: vec![tx.clone()],
        metadata: None,
                total_gas_used: 0,
                signer: None,
                signature: None,
                quorum_signatures: Vec::new(),
            };

    // Sign the block with consensus key
    let consensus = PoAConsensus::new(consensus_pk, 1000).with_signing_key(consensus_sk);
    consensus.sign_block(&mut block).unwrap();

    // Apply block directly via ledger
    runtime.ledger().apply_block(&block).unwrap();

    // Check that the transaction status is recorded as Failed
    let status = runtime.storage().get_transaction_status(&tx.hash).unwrap();
    assert!(status.is_some(), "CQ-11: Transaction status should be recorded in storage");
    match status.unwrap() {
        TransactionStatus::Failed(_) => {} // expected
        other => panic!("CQ-11: Expected Failed status, got {:?}", other),
    }
}

// CQ-11: Successful transaction status recorded as Success
#[test]
fn cq11_success_tx_status_recorded() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (sk, pk) = make_test_account(&runtime, 1_000_000);

    let tx = make_transfer_tx(pk, &sk, pk, 100, 1, 21_000, 1, 1);
    runtime.submit_transaction(tx.clone()).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let _block = tokio_rt.block_on(runtime.produce_block()).unwrap();

    let status = runtime.storage().get_transaction_status(&tx.hash).unwrap();
    assert!(status.is_some(), "CQ-11: Transaction status should be recorded in storage");
    match status.unwrap() {
        TransactionStatus::Success => {} // expected
        other => panic!("CQ-11: Expected Success status, got {:?}", other),
    }
}

// CQ-12: Duplicate transaction rejected in block validation
#[test]
fn cq12_duplicate_tx_rejected_in_block() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let storage = SledStorage::new(temp_dir.path()).unwrap();
    let consensus_sk =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_sk.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(consensus_sk);
    let sync_layer = NoopSync;
    let runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.start().unwrap();

    let (sk, pk) = make_test_account(&runtime, 1_000_000);
    let tx = make_transfer_tx(pk, &sk, pk, 100, 1, 21_000, 1, 1);
    runtime.submit_transaction(tx.clone()).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let block1 = tokio_rt.block_on(runtime.produce_block()).unwrap();
    assert_eq!(block1.transactions.len(), 1);

    // Manually create block2 with the SAME transaction hash (duplicate)
    let mut block2 = Block {
        index: block1.index + 1,
        timestamp: block1.timestamp + 1,
        prev_hash: block1.hash,
        state_root: [0u8; 32],
        hash: [0u8; 32],
        nonce: 0,
        transactions: vec![tx.clone()],
        metadata: None,
                total_gas_used: 0,
                signer: None,
                signature: None,
                quorum_signatures: Vec::new(),
            };
    block2.hash = block2.calculate_hash().unwrap();

    let result = runtime.ledger().validate_block(&block2);
    assert!(
        result.is_err(),
        "CQ-12: validate_block should reject block containing already-committed transaction"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Duplicate transaction"),
        "Expected Duplicate transaction error, got: {}",
        err_msg
    );
}

// CQ-2: gas_used capped at gas_limit for fee computation
#[test]
fn cq2_gas_fee_capped_at_gas_limit() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (sk, pk) = make_test_account(&runtime, 1_000_000);

    // Use a very high gas_limit so the tx is accepted, but with gas_price=1
    // Even if internal gas metering would exceed gas_limit, the fee is capped
    let tx = make_transfer_tx(pk, &sk, pk, 100, 1, 21_000, 1, 1);
    runtime.submit_transaction(tx.clone()).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let _block = tokio_rt.block_on(runtime.produce_block()).unwrap();

    let account_after = runtime.get_account(&pk).unwrap().unwrap();
    // Fee = gas_price * gas_limit = 1 * 21_000 = 21_000
    // Final balance = 1_000_000 - 100 - 21_000 = 978_900
    assert_eq!(account_after.balance(), 978_900, "CQ-2: Gas fee should be capped at gas_limit");
}

// CQ-17: state_root included in block hash
#[test]
fn cq17_state_root_affects_block_hash() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (sk, pk) = make_test_account(&runtime, 1_000_000);

    let tx = make_transfer_tx(pk, &sk, pk, 100, 1, 21_000, 1, 1);
    runtime.submit_transaction(tx.clone()).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let block1 = tokio_rt.block_on(runtime.produce_block()).unwrap();

    // Changing state_root should change block hash
    let mut block_modified = block1.clone();
    block_modified.state_root = [0xFFu8; 32];
    let hash_original = block1.calculate_hash().unwrap();
    let hash_modified = block_modified.calculate_hash().unwrap();
    assert_ne!(
        hash_original, hash_modified,
        "CQ-17: Changing state_root should change block hash"
    );
}

// CQ-7: Authorized signers persisted to storage
#[test]
fn cq7_authorized_signers_persisted() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    // First runtime instance
    let (runtime, _consensus_sk, consensus_pk) = make_runtime(&data_dir);
    let (_sk2, pk2) = make_test_account(&runtime, 1_000_000);

    // Persist signers directly via storage
    runtime.storage().put_authorized_signers(&[consensus_pk, pk2]).unwrap();

    // Stop runtime and explicitly drop it to release the database lock
    runtime.stop().unwrap();
    drop(runtime);
    // Give Windows time to release file handles (Sled background threads)
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Create new runtime with same storage â€” signers should be reloaded
    let storage2 = SledStorage::new(&data_dir).unwrap();
    let consensus_sk2 =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key2 = PublicKey::from(consensus_sk2.verifying_key());
    let _contract_engine2 = BaaLSContractEngine::new(storage2.clone()).unwrap();
    let mut consensus2 = PoAConsensus::new(test_key2, 1000).with_signing_key(consensus_sk2);
    consensus2.load_authorized_signers_from_storage(&storage2).unwrap();

    // Verify both signers were loaded
    assert!(
        consensus2.authorized_signers().contains(&consensus_pk),
        "CQ-7: Primary signer should be persisted and reloaded"
    );
    assert!(
        consensus2.authorized_signers().contains(&pk2),
        "CQ-7: Added signer should be persisted and reloaded"
    );
}

// CQ-1: contract call value must not transfer when call fails
#[test]
fn cq1_contract_call_value_not_transferred_on_failure() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (sk, pk) = make_test_account(&runtime, 1_000_000);

    // Deploy a test module whose exported method list does not include "main".
    // Calling "does_not_exist" should fail because no fallback export matches.
    let wasm = wasm_fixtures::make_storage_write_read_module();
    let contract_id = runtime.deploy_contract(&pk, &wasm, None, 500_000).unwrap();
    let contract_pk = contract_account_public_key(&contract_id);
    if runtime.get_account(&contract_pk).unwrap().is_none() {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&wasm);
        let code_hash: [u8; 32] = hasher.finalize().into();
        runtime
            .create_account(
                &contract_pk,
                Account::Contract { balance: 0, code_hash, storage_root_hash: [0u8; 32], nonce: 0 },
            )
            .unwrap();
    }
    let contract_before =
        runtime.get_account(&contract_pk).unwrap().map(|a| a.balance()).unwrap_or(0);

    let tx = make_contract_call_tx(
        pk,
        &sk,
        contract_id,
        "does_not_exist",
        vec![],
        Some(500_000),
        1,
        100_000,
        1,
        1,
    );
    runtime.submit_transaction(tx.clone()).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let _ = tokio_rt.block_on(runtime.produce_block()).unwrap();

    let status = runtime.storage().get_transaction_status(&tx.hash).unwrap();
    assert!(
        matches!(status, Some(TransactionStatus::Failed(_))),
        "CQ-1: failed contract call should be recorded as Failed, got: {:?}",
        status
    );

    let contract_after =
        runtime.get_account(&contract_pk).unwrap().map(|a| a.balance()).unwrap_or(0);
    assert_eq!(
        contract_before, contract_after,
        "CQ-1: contract balance must not increase when call fails"
    );

    let sender_after = runtime.get_account(&pk).unwrap().unwrap().balance();
    assert!(
        sender_after > 800_000,
        "CQ-1: sender should not lose call value on failed execution (balance={})",
        sender_after
    );
}

#[test]
fn phase2_tx_status_and_finality_progress_with_confirmations() {
    init_logging();
    let temp_dir = TempDir::new().unwrap();
    let (mut runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    runtime.finality_depth = 2;

    let (sk, pk) = make_test_account(&runtime, 2_000_000);
    let tx1 = make_transfer_tx(pk, &sk, pk, 1, 1, 21_000, 1, 1);
    runtime.submit_transaction(tx1.clone()).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let _block1 = tokio_rt.block_on(runtime.produce_block()).unwrap();

    let status1 = runtime.get_transaction_status(&tx1.hash).unwrap();
    assert_eq!(status1, Some(TransactionStatus::Success));
    let finality1 = runtime.get_transaction_finality(&tx1.hash).unwrap().unwrap();
    assert_eq!(finality1.confirmations, 1);
    assert!(!finality1.is_final);
    assert_eq!(finality1.status, TransactionStatus::Success);
    assert!(runtime.storage().get_transaction_by_id(&tx1.hash).unwrap().is_some());

    // Produce one more block so tx1 crosses finality depth.
    let tx2 = make_transfer_tx(pk, &sk, pk, 1, 2, 21_000, 1, 1);
    runtime.submit_transaction(tx2).unwrap();
    let _block2 = tokio_rt.block_on(runtime.produce_block()).unwrap();

    let finality2 = runtime.get_transaction_finality(&tx1.hash).unwrap().unwrap();
    assert_eq!(finality2.confirmations, 2);
    assert!(finality2.is_final);
}

