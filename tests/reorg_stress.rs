use baals::{
    storage::Storage,
    Account, Address, BaaLSContractEngine, NoopSync, PoAConsensus, PublicKey, Runtime,
    SledStorage, Transaction, TransactionPayload, TransactionSignature,
};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use tempfile::TempDir;

fn make_key() -> (SigningKey, PublicKey) {
    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(&mut secret);
    let sk = SigningKey::from_bytes(&secret);
    let pk = PublicKey::from(sk.verifying_key());
    (sk, pk)
}

fn make_runtime(dir: &std::path::Path) -> Runtime<SledStorage, PoAConsensus, NoopSync> {
    let (sk, pk) = make_key();
    let storage = SledStorage::new(dir).unwrap();
    let engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(pk, 1000).with_signing_key(sk);
    Runtime::with_mempool_limit(storage, consensus, engine, NoopSync, 10_000).unwrap()
}

/// Apply one block containing a transfer, then rollback it and verify balances restored.
#[test]
fn test_rollback_single_block() {
    let tmp = TempDir::new().unwrap();
    let rt = make_runtime(tmp.path());
    rt.start().unwrap();

    let (sk1, pk1) = make_key();
    let (_, pk2) = make_key();
    rt.create_account(&pk1, Account::Wallet { balance: 1_000_000, nonce: 0 }).unwrap();
    rt.create_account(&pk2, Account::Wallet { balance: 0, nonce: 0 }).unwrap();

    // Build and submit a transfer tx
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: pk1,
        recipient: Address::Wallet(pk2),
        payload: TransactionPayload::Transfer { amount: 500_000 },
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
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(&sk1).unwrap();
    rt.submit_transaction(tx).unwrap();

    // Produce block 1
    let rt2 = tokio::runtime::Runtime::new().unwrap();
    let block = rt2.block_on(rt.produce_block()).unwrap();
    assert_eq!(block.index, 1);

    // Verify transfer took effect (gas_price=1 * gas_used=21_000 = 21_000 fee)
    assert_eq!(
        rt.get_account(&pk1).unwrap().unwrap().balance(),
        479_000, // 1_000_000 - 500_000 (transfer) - 21_000 (gas)
        "sender balance after transfer and gas"
    );
    assert_eq!(
        rt.get_account(&pk2).unwrap().unwrap().balance(),
        500_000,
        "recipient balance should be 500_000 after transfer"
    );

    // Load and apply rollback log
    let log = rt.storage().get_rollback_log(&block.hash).unwrap();
    assert!(log.is_some(), "rollback log should exist for block 1");
    rt.storage().apply_rollback_log(&log.unwrap()).unwrap();

    // Verify balances restored to pre-block state
    assert_eq!(
        rt.get_account(&pk1).unwrap().unwrap().balance(),
        1_000_000,
        "sender balance restored to 1_000_000"
    );
    assert_eq!(
        rt.get_account(&pk2).unwrap().unwrap().balance(),
        0,
        "recipient balance restored to 0"
    );
}

/// Roll back two blocks sequentially, verify state matches genesis.
#[test]
fn test_rollback_two_blocks() {
    let tmp = TempDir::new().unwrap();
    let rt = make_runtime(tmp.path());
    rt.start().unwrap();

    let (sk1, pk1) = make_key();
    let (_, pk2) = make_key();
    rt.create_account(&pk1, Account::Wallet { balance: 1_000_000, nonce: 0 }).unwrap();
    rt.create_account(&pk2, Account::Wallet { balance: 0, nonce: 0 }).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();

    // Block 1: transfer 300k
    let mut tx1 = Transaction {
        hash: [0u8; 32],
        sender: pk1,
        recipient: Address::Wallet(pk2),
        payload: TransactionPayload::Transfer { amount: 300_000 },
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
    tx1.hash = tx1.calculate_hash().unwrap();
    tx1.sign(&sk1).unwrap();
    rt.submit_transaction(tx1).unwrap();
    let block1 = tokio_rt.block_on(rt.produce_block()).unwrap();

    // Block 2: transfer another 200k
    let mut tx2 = Transaction {
        hash: [0u8; 32],
        sender: pk1,
        recipient: Address::Wallet(pk2),
        payload: TransactionPayload::Transfer { amount: 200_000 },
        nonce: 2,
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
    tx2.hash = tx2.calculate_hash().unwrap();
    tx2.sign(&sk1).unwrap();
    rt.submit_transaction(tx2).unwrap();
    let block2 = tokio_rt.block_on(rt.produce_block()).unwrap();

    // After block1: pk1 = 1_000_000 - 300_000 - 21_000 = 679_000; pk2 = 300_000
    // After block2: pk1 = 679_000 - 200_000 - 21_000 = 458_000; pk2 = 500_000
    assert_eq!(rt.get_account(&pk1).unwrap().unwrap().balance(), 458_000);
    assert_eq!(rt.get_account(&pk2).unwrap().unwrap().balance(), 500_000);

    // Rollback block 2 then block 1 (reverse order)
    let log2 = rt.storage().get_rollback_log(&block2.hash).unwrap().unwrap();
    rt.storage().apply_rollback_log(&log2).unwrap();

    assert_eq!(rt.get_account(&pk1).unwrap().unwrap().balance(), 679_000, "after block2 rollback");
    assert_eq!(rt.get_account(&pk2).unwrap().unwrap().balance(), 300_000, "after block2 rollback");

    let log1 = rt.storage().get_rollback_log(&block1.hash).unwrap().unwrap();
    rt.storage().apply_rollback_log(&log1).unwrap();

    assert_eq!(
        rt.get_account(&pk1).unwrap().unwrap().balance(),
        1_000_000,
        "after both rollbacks, sender at genesis balance"
    );
    assert_eq!(
        rt.get_account(&pk2).unwrap().unwrap().balance(),
        0,
        "after both rollbacks, recipient at genesis balance"
    );
}
