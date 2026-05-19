use baals::{Block, RedbStorage, SledStorage, Storage};
use sha2::Digest;
use tempfile::TempDir;

fn sample_block(index: u64, prev_hash: [u8; 32], timestamp: u64) -> Block {
    let mut block = Block {
        index,
        timestamp,
        prev_hash,
        state_root: [0u8; 32],
        hash: [0u8; 32],
        nonce: 0,
        transactions: vec![],
        metadata: None,
    };
    block.hash = block.calculate_hash().unwrap();
    block
}

fn assert_block_checksum_is_persisted(storage: &dyn Storage) {
    let block = sample_block(1, [0u8; 32], 1_700_000_000);
    storage.put_block(&block).unwrap();

    let encoded = bincode::serialize(&block).unwrap();
    let expected_checksum: [u8; 32] = sha2::Sha256::digest(&encoded).into();
    let stored_checksum = storage
        .get_block_checksum(&block.hash)
        .unwrap()
        .expect("block checksum should be stored with the block");
    assert_eq!(stored_checksum, expected_checksum);
}

#[test]
fn sled_persists_block_checksums() {
    let tmp = TempDir::new().unwrap();
    let storage = SledStorage::new(tmp.path()).unwrap();
    assert_block_checksum_is_persisted(&storage);
}

#[test]
fn redb_persists_block_checksums() {
    let tmp = TempDir::new().unwrap();
    let storage = RedbStorage::new(tmp.path()).unwrap();
    assert_block_checksum_is_persisted(&storage);
}
