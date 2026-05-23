/// Phase K — Oracle Integration Tests
///
/// Test dormancy proof submission, attestation signing, storage, and retrieval.

#[cfg(test)]
mod phase_k_oracle_tests {
    use baals::config::OracleConfig;
    use baals::evm_submitter::EVMSubmitter;
    use baals::oracle::{
        attestation_storage_key, chain_index_key, chains_storage_key, oracle_namespace_id,
        sign_attestation, DormancyProof, OracleAttestation,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    /// Test that a DormancyProof can be verified with a valid signature.
    #[test]
    fn test_dormancy_proof_signature_verification() {
        // Generate a test signing key
        let sk = SigningKey::from_bytes(&[1u8; 32]);
        let vk = sk.verifying_key();

        // Create a dormancy proof
        let mut proof = DormancyProof {
            version: "chrononode:dormancy:v1".to_string(),
            chain_id: "bitcoin".to_string(),
            address: "1A1z7agoat7qcUeF".to_string(),
            dormant_since_block: 700000,
            current_block: 850000,
            threshold_blocks: 100000,
            signer_pubkey: Some(hex::encode(vk.to_bytes())),
            signature: None,
        };

        // Sign it
        let canonical_msg = proof.canonical_message();
        let sig = sk.sign(&canonical_msg);
        proof.signature = Some(hex::encode(sig.to_bytes()));

        // Verify signature
        assert!(proof.verify_chrononode_signature().is_ok());

        // Tamper and verify it fails
        proof.chain_id = "ethereum".to_string();
        assert!(proof.verify_chrononode_signature().is_err());
    }

    /// Test that DormancyProof with missing signature is rejected.
    #[test]
    fn test_dormancy_proof_missing_signature() {
        let proof = DormancyProof {
            version: "chrononode:dormancy:v1".to_string(),
            chain_id: "bitcoin".to_string(),
            address: "1A1z7agoat7qcUeF".to_string(),
            dormant_since_block: 700000,
            current_block: 850000,
            threshold_blocks: 100000,
            signer_pubkey: None,
            signature: None,
        };

        assert!(proof.verify_chrononode_signature().is_err());
    }

    /// Test that OracleAttestation is created with correct BaaLS signature.
    #[test]
    fn test_oracle_attestation_signing() {
        let baals_sk = SigningKey::from_bytes(&[2u8; 32]);
        let chrononode_sk = SigningKey::from_bytes(&[3u8; 32]);
        let chrononode_vk = chrononode_sk.verifying_key();

        let mut proof = DormancyProof {
            version: "chrononode:dormancy:v1".to_string(),
            chain_id: "bitcoin".to_string(),
            address: "1A1z7agoat".to_string(),
            dormant_since_block: 700000,
            current_block: 850000,
            threshold_blocks: 100000,
            signer_pubkey: Some(hex::encode(chrononode_vk.to_bytes())),
            signature: None,
        };

        let canonical_msg = proof.canonical_message();
        let chrononode_sig = chrononode_sk.sign(&canonical_msg);
        proof.signature = Some(hex::encode(chrononode_sig.to_bytes()));

        // Verify ChronoNode signature before BaaLS signs
        assert!(proof.verify_chrononode_signature().is_ok());

        let block_hash = [42u8; 32];
        let attestation = sign_attestation(proof.clone(), &baals_sk, 12345, block_hash);

        // Verify the attestation contains both signatures
        assert!(!attestation.baals_pubkey.is_empty());
        assert!(!attestation.baals_signature.is_empty());
        assert_eq!(attestation.attested_at_block, 12345);
        assert_eq!(attestation.baals_block_hash, hex::encode(block_hash));
        assert_eq!(attestation.proof.chain_id, "bitcoin");
    }

    /// Test storage key generation for attestations
    #[test]
    fn test_storage_key_generation() {
        let chain_id = "dogecoin";
        let address = "D6Z2Xp";

        let key = attestation_storage_key(chain_id, address);
        let key_str = String::from_utf8(key).unwrap();
        assert_eq!(key_str, "oracle:attest:dogecoin:D6Z2Xp");

        let index_key = chain_index_key(chain_id);
        let index_key_str = String::from_utf8(index_key).unwrap();
        assert_eq!(index_key_str, "oracle:index:dogecoin");

        let chains_key_str = String::from_utf8(chains_storage_key()).unwrap();
        assert_eq!(chains_key_str, "oracle:chains");
    }

    /// Test oracle namespace ID is deterministic
    #[test]
    fn test_oracle_namespace_id_deterministic() {
        let ns1 = oracle_namespace_id();
        let ns2 = oracle_namespace_id();
        assert_eq!(ns1, ns2);

        // Verify it's derived from SHA256
        let mut h = Sha256::new();
        h.update(b"baals:oracle:v1");
        let expected_hash = h.finalize();
        let mut expected = [0u8; 32];
        expected.copy_from_slice(&expected_hash);
        assert_eq!(ns1, expected);
    }

    /// Test EVM submitter creation and configuration
    #[test]
    fn test_evm_submitter_init() {
        let config = OracleConfig {
            enabled: true,
            evm_rpc: "https://arb-sepolia.g.alchemy.com/v2/demo".to_string(),
            reward_distributor: "0xabcd1234567890abcd1234567890abcd12345678".to_string(),
            amoy_rpc: "https://rpc-amoy.polygon.technology/".to_string(),
            evm_private_key_env: "BAALS_EVM_PRIVATE_KEY".to_string(),
            evm_chain_id: 421614,
        };

        let evm_key = vec![42u8; 32];
        let _submitter = EVMSubmitter::new(config.clone(), evm_key);

        // Verify config is accessible (this is a basic sanity check)
        assert!(config.enabled);
        assert_eq!(config.evm_chain_id, 421614);
    }

    /// Test dormancy proof with various chain IDs
    #[test]
    fn test_dormancy_proof_multiple_chains() {
        let sk = SigningKey::from_bytes(&[1u8; 32]);
        let vk = sk.verifying_key();

        let chains = vec!["bitcoin", "dogecoin", "ethereum", "monero"];

        for chain in chains {
            let mut proof = DormancyProof {
                version: "chrononode:dormancy:v1".to_string(),
                chain_id: chain.to_string(),
                address: format!("addr_{}", chain),
                dormant_since_block: 700000,
                current_block: 850000,
                threshold_blocks: 100000,
                signer_pubkey: Some(hex::encode(vk.to_bytes())),
                signature: None,
            };

            let canonical_msg = proof.canonical_message();
            let sig = sk.sign(&canonical_msg);
            proof.signature = Some(hex::encode(sig.to_bytes()));

            // Should verify successfully for each chain
            assert!(
                proof.verify_chrononode_signature().is_ok(),
                "Failed to verify proof for chain: {}",
                chain
            );
        }
    }

    /// Test canonical message format consistency
    #[test]
    fn test_canonical_message_format() {
        let proof = DormancyProof {
            version: "chrononode:dormancy:v1".to_string(),
            chain_id: "bitcoin".to_string(),
            address: "1A1z".to_string(),
            dormant_since_block: 100,
            current_block: 200,
            threshold_blocks: 50,
            signer_pubkey: None,
            signature: None,
        };

        let canonical = proof.canonical_message();

        // Verify format contains all components
        let canonical_str = String::from_utf8_lossy(&canonical);
        assert!(canonical_str.contains("chrononode:dormancy:v1"));
        assert!(canonical_str.contains("bitcoin"));
        assert!(canonical_str.contains("1A1z"));
    }

    /// Test that two identical proofs produce the same canonical message
    #[test]
    fn test_canonical_message_deterministic() {
        let proof1 = DormancyProof {
            version: "chrononode:dormancy:v1".to_string(),
            chain_id: "bitcoin".to_string(),
            address: "1A1z".to_string(),
            dormant_since_block: 100,
            current_block: 200,
            threshold_blocks: 50,
            signer_pubkey: None,
            signature: None,
        };

        let proof2 = DormancyProof {
            version: "chrononode:dormancy:v1".to_string(),
            chain_id: "bitcoin".to_string(),
            address: "1A1z".to_string(),
            dormant_since_block: 100,
            current_block: 200,
            threshold_blocks: 50,
            signer_pubkey: None,
            signature: None,
        };

        let msg1 = proof1.canonical_message();
        let msg2 = proof2.canonical_message();
        assert_eq!(msg1, msg2);
    }

    /// Test attestation serialization/deserialization
    #[test]
    fn test_attestation_serialization() {
        let baals_sk = SigningKey::from_bytes(&[2u8; 32]);
        let chrononode_sk = SigningKey::from_bytes(&[3u8; 32]);
        let chrononode_vk = chrononode_sk.verifying_key();

        let mut proof = DormancyProof {
            version: "chrononode:dormancy:v1".to_string(),
            chain_id: "bitcoin".to_string(),
            address: "1A1z7agoat".to_string(),
            dormant_since_block: 700000,
            current_block: 850000,
            threshold_blocks: 100000,
            signer_pubkey: Some(hex::encode(chrononode_vk.to_bytes())),
            signature: None,
        };

        let canonical_msg = proof.canonical_message();
        let chrononode_sig = chrononode_sk.sign(&canonical_msg);
        proof.signature = Some(hex::encode(chrononode_sig.to_bytes()));

        let block_hash = [42u8; 32];
        let attestation = sign_attestation(proof, &baals_sk, 12345, block_hash);

        // Serialize
        let json = serde_json::to_string(&attestation).unwrap();
        assert!(!json.is_empty());

        // Deserialize
        let deserialized: OracleAttestation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.attested_at_block, attestation.attested_at_block);
        assert_eq!(deserialized.baals_pubkey, attestation.baals_pubkey);
    }
}
