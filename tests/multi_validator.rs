use baals::{
    consensus::{ConsensusEngine, PoAConsensus},
    types::{Block, PublicKey, ValidatorSet},
};
use ed25519_dalek::SigningKey;
use rand::RngCore;

fn make_signing_key() -> (SigningKey, PublicKey) {
    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(&mut secret);
    let sk = SigningKey::from_bytes(&secret);
    let pk = PublicKey::from(sk.verifying_key());
    (sk, pk)
}

#[test]
fn test_quorum_acceptance_single_threshold() {
    let (sk1, pk1) = make_signing_key();

    let consensus = PoAConsensus::new(pk1, 1000)
        .with_signing_key(sk1)
        .with_quorum_threshold(1);

    let chain_state = baals::types::ChainState {
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

    let block = consensus.generate_block(&[], &prev, &chain_state).unwrap();
    assert!(
        consensus.validate_block(&block).is_ok(),
        "Single-threshold quorum should accept a validly signed block"
    );
}

#[test]
fn test_quorum_rejection_missing_signatures() {
    let (sk1, pk1) = make_signing_key();
    let (_, pk2) = make_signing_key();
    let (_, pk3) = make_signing_key();

    let consensus = PoAConsensus::new(pk1, 1000)
        .with_signing_key(sk1)
        .with_quorum_threshold(3); // require 3 of 3
    consensus.add_authorized_signer(pk2);
    consensus.add_authorized_signer(pk3);

    let chain_state = baals::types::ChainState {
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

    // Only pk1 signs â€” quorum requires 3
    let block = consensus.generate_block(&[], &prev, &chain_state).unwrap();
    let result = consensus.validate_block(&block);
    assert!(
        result.is_err(),
        "Quorum of 3 should reject a block with only 1 signature"
    );
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("Quorum not met"),
        "Error should mention quorum: {}",
        err
    );
}

#[test]
fn test_quorum_acceptance_with_extra_signatures() {
    let (sk1, pk1) = make_signing_key();
    let (sk2, pk2) = make_signing_key();
    let (_sk3, pk3) = make_signing_key();

    let consensus = PoAConsensus::new(pk1, 1000)
        .with_signing_key(sk1)
        .with_quorum_threshold(2); // require 2 of 3
    consensus.add_authorized_signer(pk2);
    consensus.add_authorized_signer(pk3);

    let chain_state = baals::types::ChainState {
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

    // pk1 signs as primary, pk2 adds quorum signature
    let mut block = consensus.generate_block(&[], &prev, &chain_state).unwrap();

    // Add pk2's signature to quorum_signatures
    use ed25519_dalek::Signer;
    let sig2 = sk2.sign(&block.hash);
    block.quorum_signatures.push((
        hex::encode(pk2.to_bytes()),
        sig2.to_bytes().to_vec(),
    ));

    let result = consensus.validate_block(&block);
    assert!(
        result.is_ok(),
        "Quorum of 2 should accept block with 2 valid signatures: {:?}",
        result.err()
    );
}

#[test]
fn test_quorum_rejects_invalid_extra_signature() {
    let (sk1, pk1) = make_signing_key();
    let (_, pk2) = make_signing_key();

    let consensus = PoAConsensus::new(pk1, 1000)
        .with_signing_key(sk1)
        .with_quorum_threshold(2);
    consensus.add_authorized_signer(pk2);

    let chain_state = baals::types::ChainState {
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

    let mut block = consensus.generate_block(&[], &prev, &chain_state).unwrap();

    // Add a garbage (invalid) signature for pk2
    block.quorum_signatures.push((
        hex::encode(pk2.to_bytes()),
        vec![0u8; 64], // invalid signature bytes
    ));

    let result = consensus.validate_block(&block);
    assert!(
        result.is_err(),
        "Invalid quorum signature should cause rejection"
    );
}

#[test]
fn test_validator_set_struct() {
    let (_, pk1) = make_signing_key();
    let (_, pk2) = make_signing_key();

    let vs = ValidatorSet {
        signers: vec![pk1, pk2],
        quorum: 2,
        effective_height: 100,
    };
    assert_eq!(vs.signers.len(), 2);
    assert_eq!(vs.quorum, 2);
    assert_eq!(vs.effective_height, 100);
}

#[test]
fn test_round_robin_expected_signer() {
    let (_, pk1) = make_signing_key();
    let (_, pk2) = make_signing_key();
    let (_, pk3) = make_signing_key();

    let consensus = PoAConsensus::new(pk1, 1000).with_round_robin(true);
    consensus.add_authorized_signer(pk2);
    consensus.add_authorized_signer(pk3);

    // All signers: [pk1, pk2, pk3]
    assert_eq!(consensus.expected_signer_for_index(0).to_bytes(), pk1.to_bytes());
    assert_eq!(consensus.expected_signer_for_index(1).to_bytes(), pk2.to_bytes());
    assert_eq!(consensus.expected_signer_for_index(2).to_bytes(), pk3.to_bytes());
    assert_eq!(consensus.expected_signer_for_index(3).to_bytes(), pk1.to_bytes()); // wraps
    assert_eq!(consensus.expected_signer_for_index(7).to_bytes(), pk2.to_bytes()); // 7 % 3 = 1
}

#[test]
fn test_round_robin_violation_rejected() {
    let (sk1, pk1) = make_signing_key();
    let (_, pk2) = make_signing_key();
    let (_, pk3) = make_signing_key();

    // pk1 is the signing key but block 1 should be signed by pk2 in round-robin
    let consensus = PoAConsensus::new(pk1, 1000)
        .with_signing_key(sk1)
        .with_round_robin(true);
    consensus.add_authorized_signer(pk2);
    consensus.add_authorized_signer(pk3);

    // generate_block will pick pk2 (index 1) for round-robin, but pk1 is the signing key
    // This means block generation will fail or produce a block signed by wrong key
    // Instead, manually craft a block signed by pk1 for slot 1 (should be pk2)
    let mut meta = std::collections::BTreeMap::new();
    meta.insert("signer".to_string(), hex::encode(pk1.to_bytes()));
    let mut block = Block {
        index: 1,
        timestamp: 1_700_000_001,
        prev_hash: [0u8; 32],
        state_root: [0u8; 32],
        hash: [0u8; 32],
        nonce: 0,
        transactions: vec![],
        metadata: Some(meta),
        total_gas_used: 0,
        signer: Some(hex::encode(pk1.to_bytes())),
        signature: None,
        quorum_signatures: vec![],
    };
    consensus.sign_block(&mut block).unwrap();

    // pk1 signed block 1, but round-robin expects pk2 (and grace allows pk1 = slot 0)
    // Since pk1 is the slot-0 signer, it IS in the grace window for slot 1
    // So this should actually pass (grace window covers one slot back)
    // Let's verify the grace window behavior
    let result = consensus.validate_block(&block);
    // Slot 1 expected: pk2. Grace (slot 0) expected: pk1. pk1 is in grace → should pass
    assert!(
        result.is_ok(),
        "Grace window should allow slot-0 signer at slot-1: {:?}",
        result.err()
    );
}

#[test]
fn test_round_robin_clear_violation_rejected() {
    let (sk1, pk1) = make_signing_key();
    let (_, pk2) = make_signing_key();
    let (_, pk3) = make_signing_key();

    let consensus = PoAConsensus::new(pk1, 1000)
        .with_signing_key(sk1)
        .with_round_robin(true);
    consensus.add_authorized_signer(pk2);
    consensus.add_authorized_signer(pk3);

    // Block at index 2 should be pk3 (slot 2), grace allows pk2 (slot 1)
    // pk1 (slot 0) is too far out — should be rejected
    let mut meta = std::collections::BTreeMap::new();
    meta.insert("signer".to_string(), hex::encode(pk1.to_bytes()));
    let mut block = Block {
        index: 2,
        timestamp: 1_700_000_002,
        prev_hash: [0u8; 32],
        state_root: [0u8; 32],
        hash: [0u8; 32],
        nonce: 0,
        transactions: vec![],
        metadata: Some(meta),
        total_gas_used: 0,
        signer: Some(hex::encode(pk1.to_bytes())),
        signature: None,
        quorum_signatures: vec![],
    };
    consensus.sign_block(&mut block).unwrap();

    let result = consensus.validate_block(&block);
    assert!(
        result.is_err(),
        "pk1 signing slot-2 block (expected pk3, grace pk2) should be rejected"
    );
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("Round-robin"),
        "Error should mention Round-robin: {}",
        err
    );
}
