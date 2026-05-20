use baals::{
    consensus::{ConsensusEngine, PoAConsensus},
    contracts::BaaLSContractEngine,
    ledger::Ledger,
    storage::{SledStorage, Storage},
    types::{
        Account, Address, Block, ChainState, PublicKey, Transaction, TransactionPayload,
        TransactionSignature,
    },
};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use std::sync::Arc;
use tempfile::TempDir;

fn make_key() -> (SigningKey, PublicKey) {
    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(&mut secret);
    let sk = SigningKey::from_bytes(&secret);
    let pk = PublicKey::from(sk.verifying_key());
    (sk, pk)
}

#[test]
fn test_equivocation_detection() {
    let tmp = TempDir::new().unwrap();
    let raw_storage = SledStorage::new(tmp.path()).unwrap();
    let engine = BaaLSContractEngine::new(raw_storage.clone()).unwrap();
    let storage = Arc::new(raw_storage);
    let ledger = Ledger::new(storage.clone(), Arc::new(engine));
    ledger.initialize_chain().unwrap();

    let (sk, pk) = make_key();

    storage.put_account(&pk, &Account::Wallet { balance: 1_000_000, nonce: 0 }).unwrap();

    let chain_state = storage.get_chain_state().unwrap().unwrap();
    let genesis = storage.get_block(&chain_state.latest_block_hash).unwrap().unwrap();

    let mut tx_a = Transaction {
        hash: [0u8; 32],
        sender: pk,
        recipient: Address::Wallet(pk),
        payload: TransactionPayload::Data { data: vec![1, 2, 3] },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        gas_price: 1,
        priority: 0,
        metadata: None,
        chain_id: 1,
    };
    tx_a.hash = tx_a.calculate_hash().unwrap();
    tx_a.sign(&sk).unwrap();

    let mut tx_b = Transaction {
        hash: [0u8; 32],
        sender: pk,
        recipient: Address::Wallet(pk),
        payload: TransactionPayload::Data { data: vec![4, 5, 6] },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100_000,
        gas_price: 1,
        priority: 0,
        metadata: None,
        chain_id: 1,
    };
    tx_b.hash = tx_b.calculate_hash().unwrap();
    tx_b.sign(&sk).unwrap();

    let consensus = PoAConsensus::new(pk, 1000).with_signing_key(sk);

    let block_a = consensus.generate_block(&[tx_a], &genesis, &chain_state).unwrap();
    assert_eq!(block_a.index, 1);
    let block_b = consensus.generate_block(&[tx_b], &genesis, &chain_state).unwrap();
    assert_eq!(block_b.index, 1);
    assert_ne!(block_a.hash, block_b.hash, "conflicting blocks must have different hashes");

    ledger.apply_block(&block_a).unwrap();
    let result = ledger.apply_block(&block_b);
    assert!(result.is_err(), "equivocation block must be rejected");
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("Invalid block index"), "error: {}", err);
}

#[test]
fn test_invalid_block_rejected() {
    let (sk_good, pk_good) = make_key();
    let (sk_bad, pk_bad) = make_key();

    let consensus = PoAConsensus::new(pk_good, 1000).with_signing_key(sk_good);
    let bad_consensus = PoAConsensus::new(pk_bad, 1000).with_signing_key(sk_bad);

    let chain_state = ChainState {
        latest_block_hash: [0u8; 32],
        latest_block_index: 0,
        accounts_root_hash: [0u8; 32],
        total_supply: 0,
    };
    let prev = Block {
        index: 0,
        timestamp: 1_700_000_000,
        prev_hash: [0u8; 32],
        state_root: [0u8; 32],
        hash: [0u8; 32],
        nonce: 0,
        transactions: vec![],
        metadata: None,
        total_gas_used: 0,
        signer: None,
        signature: None,
        quorum_signatures: vec![],
    };

    let bad_block = bad_consensus.generate_block(&[], &prev, &chain_state).unwrap();
    let result = consensus.validate_block(&bad_block);
    assert!(result.is_err(), "block signed by unauthorized signer should be rejected");
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("Unauthorized"), "error should mention Unauthorized: {}", err);
}

#[test]
fn test_malformed_transaction_rejected() {
    let tmp = TempDir::new().unwrap();
    let raw_storage = SledStorage::new(tmp.path()).unwrap();
    let engine = BaaLSContractEngine::new(raw_storage.clone()).unwrap();
    let storage = Arc::new(raw_storage);
    let ledger = Ledger::new(storage.clone(), Arc::new(engine));
    ledger.initialize_chain().unwrap();

    let (sk, pk) = make_key();

    storage.put_account(&pk, &Account::Wallet { balance: 1_000_000, nonce: 0 }).unwrap();

    let mut bad_tx = Transaction {
        hash: [0u8; 32],
        sender: pk,
        recipient: Address::Wallet(pk),
        payload: TransactionPayload::ContractCall {
            method: "test".to_string(),
            args: vec![],
            value: None,
        },
        nonce: 1,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 200_000,
        gas_price: 1,
        priority: 0,
        metadata: None,
        chain_id: 1,
    };
    bad_tx.hash = bad_tx.calculate_hash().unwrap();
    bad_tx.sign(&sk).unwrap();

    let chain_state = storage.get_chain_state().unwrap().unwrap();
    let genesis = storage.get_block(&chain_state.latest_block_hash).unwrap().unwrap();

    let consensus = PoAConsensus::new(pk, 1000).with_signing_key(sk);

    let block = consensus.generate_block(&[bad_tx], &genesis, &chain_state).unwrap();
    let result = ledger.apply_block(&block);
    assert!(result.is_err(), "malformed transaction should be rejected");
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("Invalid transaction payload"),
        "error should mention Invalid transaction payload: {}",
        err
    );
}
