use std::sync::Arc;

use baals::{
    contracts::BaaLSContractEngine,
    ledger::Ledger,
    storage::Storage,
    types::{Block, ChainState},
    Account, Address, PublicKey, SledStorage, Transaction, TransactionPayload,
    TransactionSignature,
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

fn create_signed_tx(
    sk: &SigningKey,
    sender: PublicKey,
    recipient: PublicKey,
    amount: u64,
    nonce: u64,
) -> Transaction {
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender,
        recipient: Address::Wallet(recipient),
        payload: TransactionPayload::Transfer { amount },
        nonce,
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
    tx.sign(sk).unwrap();
    tx
}

fn make_ledger(
    dir: &std::path::Path,
) -> (Arc<SledStorage>, Ledger<SledStorage, BaaLSContractEngine<SledStorage>>) {
    let storage_raw = SledStorage::new(dir).unwrap();
    let engine = BaaLSContractEngine::new(storage_raw.clone()).unwrap();
    let storage = Arc::new(storage_raw);
    let ledger = Ledger::new(Arc::clone(&storage), Arc::new(engine));
    (storage, ledger)
}

fn setup_chain_with_accounts(
    storage: &SledStorage,
    ledger: &Ledger<SledStorage, BaaLSContractEngine<SledStorage>>,
    pk1: &PublicKey,
    pk2: &PublicKey,
) -> [u8; 32] {
    ledger.initialize_chain().unwrap();
    storage.put_account(pk1, &Account::Wallet { balance: 1_000_000, nonce: 0 }).unwrap();
    storage.put_account(pk2, &Account::Wallet { balance: 0, nonce: 0 }).unwrap();
    storage.get_chain_state().unwrap().map(|cs: ChainState| cs.latest_block_hash).unwrap()
}

/// Build a block with given transactions and apply it via the ledger.
fn apply_block(
    ledger: &Ledger<SledStorage, BaaLSContractEngine<SledStorage>>,
    storage: &SledStorage,
    index: u64,
    prev_hash: [u8; 32],
    transactions: Vec<Transaction>,
) -> Block {
    let state_root = storage
        .get_chain_state()
        .unwrap()
        .map(|cs: ChainState| cs.accounts_root_hash)
        .unwrap_or([0u8; 32]);

    let mut block = Block {
        index,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        prev_hash,
        state_root,
        hash: [0u8; 32],
        nonce: 0,
        transactions,
        metadata: None,
        total_gas_used: 0,
        signer: None,
        signature: None,
        quorum_signatures: vec![],
    };
    block.hash = block.calculate_hash().unwrap();
    ledger.apply_block(&block).unwrap();
    block
}

/// Apply blocks, crash (re-open storage), verify chain state is consistent.
#[test]
fn test_crash_during_apply_block() {
    let tmp = TempDir::new().unwrap();

    let (sk1, pk1) = make_key();
    let (_, pk2) = make_key();

    // --- First session ---
    let (_storage, ledger) = make_ledger(tmp.path());
    let genesis_hash = setup_chain_with_accounts(&_storage, &ledger, &pk1, &pk2);

    let tx1 = create_signed_tx(&sk1, pk1, pk2, 300_000, 1);
    let block1 = apply_block(&ledger, &_storage, 1, genesis_hash, vec![tx1]);
    assert_eq!(block1.index, 1);

    let tx2 = create_signed_tx(&sk1, pk1, pk2, 200_000, 2);
    let block2 = apply_block(&ledger, &_storage, 2, block1.hash, vec![tx2]);
    assert_eq!(block2.index, 2);

    // Drop ledger+storage — sled lock released
    drop((_storage, ledger));

    // --- Crash simulation: re-open storage on same path ---
    let storage = SledStorage::new(tmp.path()).unwrap();
    let height = storage.get_chain_height().unwrap();
    assert_eq!(height, 2, "chain height should be 2 after crash recovery");

    let b1 = storage.get_block_by_height(1).unwrap().unwrap();
    assert_eq!(b1.index, 1);
    let b2 = storage.get_block_by_height(2).unwrap().unwrap();
    assert_eq!(b2.index, 2);

    let acc1 = storage.get_account(&pk1).unwrap().unwrap();
    let acc2 = storage.get_account(&pk2).unwrap().unwrap();
    assert_eq!(acc1.balance(), 458_000, "sender balance after two blocks");
    assert_eq!(acc2.balance(), 500_000, "recipient balance after two blocks");
}

/// Apply blocks, backup, verify backup data integrity by re-opening it.
#[test]
fn test_crash_during_backup() {
    let tmp = TempDir::new().unwrap();

    let (sk1, pk1) = make_key();
    let (_, pk2) = make_key();

    // --- First session: apply blocks and create backup ---
    let (storage, ledger) = make_ledger(tmp.path());
    let genesis_hash = setup_chain_with_accounts(&storage, &ledger, &pk1, &pk2);

    let tx1 = create_signed_tx(&sk1, pk1, pk2, 100_000, 1);
    apply_block(&ledger, &storage, 1, genesis_hash, vec![tx1]);

    // Create backup while ledger is still alive
    let backup_dir = tmp.path().join("backup");
    storage.backup_to(&backup_dir).unwrap();

    // Drop original storage+ledger so the backup sled lock is released
    drop((storage, ledger));

    // Restore from backup into a fresh storage (simulating disaster recovery)
    let restore_dir = TempDir::new().unwrap();
    let restore_storage = SledStorage::new(restore_dir.path()).unwrap();
    restore_storage.restore_from(&backup_dir).unwrap();
    let height = restore_storage.get_chain_height().unwrap();
    assert_eq!(height, 1, "restored storage should have block 1");
    let restored_pk1 = restore_storage.get_account(&pk1).unwrap().unwrap();
    assert_eq!(restored_pk1.balance(), 879_000, "restored balance after 1 block");

    // Also open the backup directly to verify it's a valid sled DB
    let backup_storage = SledStorage::new(&backup_dir).unwrap();
    let height = backup_storage.get_chain_height().unwrap();
    assert_eq!(height, 1, "backup should contain blocks");
    let acc1 = backup_storage.get_account(&pk1).unwrap().unwrap();
    assert_eq!(acc1.balance(), 879_000, "backup should have account data");
}

/// Apply blocks, crash, restart, verify rollback logs survive and can undo blocks.
#[test]
fn test_rollback_log_survives_crash() {
    let tmp = TempDir::new().unwrap();

    let (sk1, pk1) = make_key();
    let (_, pk2) = make_key();

    // --- First session ---
    let block1_hash;
    let block2_hash;
    {
        let (_storage, ledger) = make_ledger(tmp.path());
        let genesis_hash = setup_chain_with_accounts(&_storage, &ledger, &pk1, &pk2);

        let tx1 = create_signed_tx(&sk1, pk1, pk2, 300_000, 1);
        let block1 = apply_block(&ledger, &_storage, 1, genesis_hash, vec![tx1]);
        block1_hash = block1.hash;
        assert_eq!(block1.index, 1);

        let tx2 = create_signed_tx(&sk1, pk1, pk2, 200_000, 2);
        let block2 = apply_block(&ledger, &_storage, 2, block1.hash, vec![tx2]);
        block2_hash = block2.hash;
        assert_eq!(block2.index, 2);
    }
    // All Arcs dropped, sled lock released.

    // --- Crash simulation: re-open storage ---
    let storage = SledStorage::new(tmp.path()).unwrap();

    // Rollback logs should survive crash
    let log1 = storage.get_rollback_log(&block1_hash).unwrap();
    assert!(log1.is_some(), "rollback log for block 1 should survive crash");
    let log2 = storage.get_rollback_log(&block2_hash).unwrap();
    assert!(log2.is_some(), "rollback log for block 2 should survive crash");

    // Apply rollback logs to undo both blocks (reverse order)
    storage.apply_rollback_log(&log2.unwrap()).unwrap();
    let acc1 = storage.get_account(&pk1).unwrap().unwrap();
    assert_eq!(acc1.balance(), 679_000, "after block2 rollback");
    let acc2 = storage.get_account(&pk2).unwrap().unwrap();
    assert_eq!(acc2.balance(), 300_000, "after block2 rollback");

    storage.apply_rollback_log(&log1.unwrap()).unwrap();
    let acc1 = storage.get_account(&pk1).unwrap().unwrap();
    assert_eq!(acc1.balance(), 1_000_000, "after both rollbacks, sender at genesis");
    let acc2 = storage.get_account(&pk2).unwrap().unwrap();
    assert_eq!(acc2.balance(), 0, "after both rollbacks, recipient at genesis");
}
