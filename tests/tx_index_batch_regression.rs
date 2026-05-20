use baals::*;
use tempfile::TempDir;

fn sample_tx(sender: PublicKey, recipient: PublicKey) -> Transaction {
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender,
        recipient: Address::Wallet(recipient),
        payload: TransactionPayload::Transfer { amount: 1 },
        nonce: 1,
        timestamp: 1,
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 21_000,
        gas_price: 1,
        priority: 0,
        metadata: None,
        chain_id: 1,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx
}

fn sample_block(tx: Transaction) -> Block {
    let mut block = Block {
        index: 1,
        timestamp: 2,
        prev_hash: [0u8; 32],
        state_root: [0u8; 32],
        hash: [0u8; 32],
        nonce: 0,
        transactions: vec![tx],
        metadata: None,
                total_gas_used: 0,
                signer: None,
                signature: None,
                quorum_signatures: Vec::new(),
            };
    block.hash = block.calculate_hash().unwrap();
    block
}

fn assert_batch_index_lookup_works(storage: &dyn Storage) {
    let sender = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let recipient = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let sender_pk = PublicKey::from(sender.verifying_key());
    let recipient_pk = PublicKey::from(recipient.verifying_key());

    let tx = sample_tx(sender_pk, recipient_pk);
    let block = sample_block(tx.clone());
    storage.put_transaction(&tx).unwrap();
    storage.put_block(&block).unwrap();

    let mut batch = StorageBatch::default();
    let by_block_key =
        format!("block_tx:{}:{}:{:0>10}", hex::encode(block.hash), hex::encode(tx.hash), 0);
    batch
        .ops
        .push(StorageOperation::PutTxIndex(by_block_key.as_bytes().to_vec(), tx.hash.to_vec()));
    let reverse_key = format!("tx_block:{}", hex::encode(tx.hash));
    batch.ops.push(StorageOperation::PutTxIndex(
        reverse_key.as_bytes().to_vec(),
        block.hash.to_vec(),
    ));
    storage.apply_batch(batch).unwrap();

    let linked = storage.get_transaction_by_id(&tx.hash).unwrap();
    assert!(linked.is_some(), "batch tx index should resolve tx->block lookup");
}

#[test]
fn batch_tx_index_lookup_sled() {
    let tmp = TempDir::new().unwrap();
    let storage = SledStorage::new(tmp.path()).unwrap();
    assert_batch_index_lookup_works(&storage);
}

#[test]
fn batch_tx_index_lookup_redb() {
    let tmp = TempDir::new().unwrap();
    let storage = RedbStorage::new(tmp.path()).unwrap();
    assert_batch_index_lookup_works(&storage);
}

