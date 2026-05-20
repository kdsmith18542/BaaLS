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

    let mut consensus = PoAConsensus::new(pk1, 1000)
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

    let mut consensus = PoAConsensus::new(pk1, 1000)
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

    let mut consensus = PoAConsensus::new(pk1, 1000)
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

