use baals::*;
use ed25519_dalek::SigningKey;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct GoldenTransaction {
    pub transaction_json: serde_json::Value,
    pub expected_hash: String,
    pub sender_pubkey: String,
    pub signature: String,
}

fn fmt_hash(h: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(h))
}

fn fmt_pk(pk: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(pk))
}

fn fmt_sig(sig: &[u8; 64]) -> String {
    format!("0x{}", hex::encode(sig))
}

fn sender1() -> SigningKey {
    SigningKey::from_bytes(&[1u8; 32])
}

fn sender2() -> SigningKey {
    SigningKey::from_bytes(&[2u8; 32])
}

fn pk1() -> PublicKey {
    PublicKey::from(sender1().verifying_key())
}

fn pk2() -> PublicKey {
    PublicKey::from(sender2().verifying_key())
}

fn known_contract() -> ContractId {
    ContractId::from_bytes(&[42u8; 32])
}

fn ts() -> u64 {
    1234567890
}

pub fn golden_vectors() -> Vec<GoldenTransaction> {
    let sk = sender1();
    let pk = pk1();
    let pk2_key = pk2();
    let t = ts();
    let contract_id = known_contract();
    let wasm = vec![
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f,
        0x01, 0x7f, 0x03, 0x02, 0x01, 0x00, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x11, 0x02, 0x06,
        0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00, 0x04, 0x6d, 0x61, 0x69, 0x6e, 0x00, 0x00,
        0x0a, 0x06, 0x01, 0x04, 0x00, 0x20, 0x01, 0x0b,
    ];
    let mut vecs = Vec::new();

    // 1. Simple transfer tx: sender sends 100
    {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: pk,
            nonce: 1,
            timestamp: t,
            recipient: Address::Wallet(pk),
            payload: TransactionPayload::Transfer { amount: 100 },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            gas_price: 0,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let h = tx.calculate_hash().unwrap();
        tx.sign(&sk).unwrap();
        vecs.push(GoldenTransaction {
            transaction_json: serde_json::to_value(&tx).unwrap(),
            expected_hash: fmt_hash(&h),
            sender_pubkey: fmt_pk(pk.as_bytes()),
            signature: fmt_sig(&tx.signature.to_bytes()),
        });
    }

    // 2. Contract deploy tx: deploys test WASM module
    {
        let sk2 = sender2();
        let pk2_key = PublicKey::from(sk2.verifying_key());
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: pk2_key,
            nonce: 1,
            timestamp: t,
            recipient: Address::Wallet(pk2_key),
            payload: TransactionPayload::ContractDeploy {
                wasm_bytes: wasm.clone(),
                init_payload: None,
            },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 500000,
            gas_price: 0,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let h = tx.calculate_hash().unwrap();
        tx.sign(&sk2).unwrap();
        vecs.push(GoldenTransaction {
            transaction_json: serde_json::to_value(&tx).unwrap(),
            expected_hash: fmt_hash(&h),
            sender_pubkey: fmt_pk(pk2_key.as_bytes()),
            signature: fmt_sig(&tx.signature.to_bytes()),
        });
    }

    // 3. Contract call tx: calls a contract with empty args
    {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: pk,
            nonce: 2,
            timestamp: t,
            recipient: Address::Contract(contract_id.clone()),
            payload: TransactionPayload::ContractCall {
                method: "test_method".into(),
                args: vec![],
                value: None,
            },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 500000,
            gas_price: 0,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let h = tx.calculate_hash().unwrap();
        tx.sign(&sk).unwrap();
        vecs.push(GoldenTransaction {
            transaction_json: serde_json::to_value(&tx).unwrap(),
            expected_hash: fmt_hash(&h),
            sender_pubkey: fmt_pk(pk.as_bytes()),
            signature: fmt_sig(&tx.signature.to_bytes()),
        });
    }

    // 4. Data tx: payload is vec![1, 2, 3, 4]
    {
        let sk2 = sender2();
        let pk2_key = PublicKey::from(sk2.verifying_key());
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: pk2_key,
            nonce: 2,
            timestamp: t,
            recipient: Address::Wallet(pk2_key),
            payload: TransactionPayload::Data { data: vec![1, 2, 3, 4] },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            gas_price: 0,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let h = tx.calculate_hash().unwrap();
        tx.sign(&sk2).unwrap();
        vecs.push(GoldenTransaction {
            transaction_json: serde_json::to_value(&tx).unwrap(),
            expected_hash: fmt_hash(&h),
            sender_pubkey: fmt_pk(pk2_key.as_bytes()),
            signature: fmt_sig(&tx.signature.to_bytes()),
        });
    }

    // 5. Tx with metadata: transfer with metadata
    {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: pk,
            nonce: 3,
            timestamp: t,
            recipient: Address::Wallet(pk),
            payload: TransactionPayload::Transfer { amount: 50 },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            gas_price: 0,
            priority: 0,
            metadata: Some(BTreeMap::from([("memo".to_string(), "test".to_string())])),
            chain_id: 1,
        };
        let h = tx.calculate_hash().unwrap();
        tx.sign(&sk).unwrap();
        vecs.push(GoldenTransaction {
            transaction_json: serde_json::to_value(&tx).unwrap(),
            expected_hash: fmt_hash(&h),
            sender_pubkey: fmt_pk(pk.as_bytes()),
            signature: fmt_sig(&tx.signature.to_bytes()),
        });
    }

    // 6. Tx with empty metadata: same as above but metadata: None
    {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: pk,
            nonce: 4,
            timestamp: t,
            recipient: Address::Wallet(pk),
            payload: TransactionPayload::Transfer { amount: 50 },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            gas_price: 0,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let h = tx.calculate_hash().unwrap();
        tx.sign(&sk).unwrap();
        vecs.push(GoldenTransaction {
            transaction_json: serde_json::to_value(&tx).unwrap(),
            expected_hash: fmt_hash(&h),
            sender_pubkey: fmt_pk(pk.as_bytes()),
            signature: fmt_sig(&tx.signature.to_bytes()),
        });
    }

    // 7. Tx with wallet recipient: explicit Address::Wallet(pk2)
    {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: pk,
            nonce: 5,
            timestamp: t,
            recipient: Address::Wallet(pk2_key),
            payload: TransactionPayload::Transfer { amount: 25 },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            gas_price: 0,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let h = tx.calculate_hash().unwrap();
        tx.sign(&sk).unwrap();
        vecs.push(GoldenTransaction {
            transaction_json: serde_json::to_value(&tx).unwrap(),
            expected_hash: fmt_hash(&h),
            sender_pubkey: fmt_pk(pk.as_bytes()),
            signature: fmt_sig(&tx.signature.to_bytes()),
        });
    }

    // 8. Tx with contract recipient: Address::Contract(known_contract)
    {
        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: pk,
            nonce: 6,
            timestamp: t,
            recipient: Address::Contract(contract_id.clone()),
            payload: TransactionPayload::Data { data: vec![9, 8, 7] },
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            gas_price: 0,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let h = tx.calculate_hash().unwrap();
        tx.sign(&sk).unwrap();
        vecs.push(GoldenTransaction {
            transaction_json: serde_json::to_value(&tx).unwrap(),
            expected_hash: fmt_hash(&h),
            sender_pubkey: fmt_pk(pk.as_bytes()),
            signature: fmt_sig(&tx.signature.to_bytes()),
        });
    }

    vecs
}

pub fn test_golden_transactions() {
    let vectors = golden_vectors();
    assert_eq!(vectors.len(), 8, "Should have 8 golden vectors");

    for (i, gv) in vectors.iter().enumerate() {
        // Deserialize transaction_json back to Transaction
        let tx: Transaction =
            serde_json::from_value(gv.transaction_json.clone()).unwrap_or_else(|e| {
                panic!("Golden vector {}: failed to deserialize Transaction: {}", i, e)
            });

        // Recalculate hash from deserialized tx
        let recomputed = tx
            .calculate_hash()
            .unwrap_or_else(|e| panic!("Golden vector {}: calculate_hash failed: {:?}", i, e));

        // Strip "0x" prefix and compare hashes
        let expected_bytes = hex::decode(&gv.expected_hash[2..])
            .unwrap_or_else(|e| panic!("Golden vector {}: invalid expected_hash hex: {}", i, e));
        assert_eq!(
            recomputed.as_slice(),
            expected_bytes.as_slice(),
            "Golden vector {}: hash mismatch.\n  Expected: {}\n  Got:      {}",
            i,
            gv.expected_hash,
            fmt_hash(&recomputed)
        );

        // Verify hash stored in transaction matches expected
        assert_eq!(
            tx.hash.as_slice(),
            expected_bytes.as_slice(),
            "Golden vector {}: stored tx.hash mismatch.\n  Expected: {}\n  Got:      {}",
            i,
            gv.expected_hash,
            fmt_hash(&tx.hash)
        );

        // Verify signature using the sender's public key
        let verify_result = tx.verify_signature().unwrap_or_else(|e| {
            panic!("Golden vector {}: verify_signature returned error: {:?}", i, e)
        });
        assert!(verify_result, "Golden vector {}: signature verification returned false", i);

        // Verify signature bytes match
        let expected_sig_bytes = hex::decode(&gv.signature[2..])
            .unwrap_or_else(|e| panic!("Golden vector {}: invalid signature hex: {}", i, e));
        assert_eq!(
            tx.signature.to_bytes().as_slice(),
            expected_sig_bytes.as_slice(),
            "Golden vector {}: signature bytes mismatch",
            i,
        );

        // Verify sender pubkey matches
        let expected_pk_bytes = hex::decode(&gv.sender_pubkey[2..])
            .unwrap_or_else(|e| panic!("Golden vector {}: invalid sender_pubkey hex: {}", i, e));
        assert_eq!(
            tx.sender.as_bytes().as_slice(),
            expected_pk_bytes.as_slice(),
            "Golden vector {}: sender pubkey mismatch",
            i,
        );

        // Cross-language compatibility: re-serialize to JSON and verify
        // the resulting JSON round-trips correctly
        let re_json = serde_json::to_value(&tx).unwrap();
        let tx2: Transaction = serde_json::from_value(re_json).unwrap();
        let hash2 = tx2.calculate_hash().unwrap();
        assert_eq!(hash2, recomputed, "Golden vector {}: JSON round-trip broke hash", i);
    }
}

pub fn test_tx_roundtrip_vec_args() {
    let sk = sender1();
    let pk = pk1();
    let t = ts();

    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: pk,
        nonce: 100,
        timestamp: t,
        recipient: Address::Wallet(pk),
        payload: TransactionPayload::ContractCall {
            method: "test".into(),
            args: vec![vec![1, 2], vec![3, 4, 5]],
            value: None,
        },
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        gas_price: 0,
        priority: 0,
        metadata: None,
        chain_id: 1,
    };

    let hash_before = tx.calculate_hash().unwrap();
    tx.sign(&sk).unwrap();
    let sig = tx.signature.to_bytes();
    let hash_after = tx.hash;

    // Verify hash didn't change between calculate and sign
    assert_eq!(
        hash_before, hash_after,
        "hash should not change between calculate_hash and sign"
    );

    // Serialize to JSON
    let json = serde_json::to_value(&tx).unwrap();

    // Deserialize back
    let tx2: Transaction =
        serde_json::from_value(json).expect("Failed to deserialize Transaction from JSON");

    // Verify hash matches
    let hash2 = tx2.calculate_hash().unwrap();
    assert_eq!(hash_after, hash2, "hash mismatch after JSON round-trip");
    assert_eq!(hash_before, hash2, "hash should be deterministic");

    // Verify signature bytes survived
    assert_eq!(tx2.signature.to_bytes(), sig, "signature mismatch after JSON round-trip");

    // Verify args are preserved as Vec<Vec<u8>>
    match &tx2.payload {
        TransactionPayload::ContractCall { method, args, value } => {
            assert_eq!(method, "test", "method name mismatch");
            assert_eq!(
                args,
                &vec![vec![1u8, 2], vec![3u8, 4, 5]],
                "args mismatch after JSON round-trip"
            );
            assert_eq!(*value, None, "value should be None");
        }
        _ => panic!("Payload type changed after round-trip: {:?}", tx2.payload),
    }

    // Verify signature still verifies
    assert!(
        tx2.verify_signature().unwrap(),
        "signature verification failed after round-trip"
    );

    // Verify sender
    assert_eq!(tx2.sender, pk, "sender mismatch after round-trip");

    // Additional cross-language: test that JSON serialized form is valid JSON
    let json_str = serde_json::to_string(&tx).unwrap();
    let _parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("JSON output should be valid");
}

#[test]
fn generate_golden_data() {
    let vectors = golden_vectors();
    assert_eq!(vectors.len(), 8);

    // Print golden vectors in a format suitable for hardcoding
    // Run with: cargo test generate_golden_data -- --nocapture
    println!("\n========== GOLDEN VECTORS ==========");
    for (i, gv) in vectors.iter().enumerate() {
        println!("// Golden vector {}", i);
        println!("GoldenTransaction {{");
        println!("    transaction_json: serde_json::json!({}),", gv.transaction_json);
        println!("    expected_hash: \"{}\",", gv.expected_hash);
        println!("    sender_pubkey: \"{}\",", gv.sender_pubkey);
        println!("    signature: \"{}\",", gv.signature);
        println!("}},");
    }
    println!("=====================================\n");

    // Self-consistency: all hashes should be nonzero
    for (i, gv) in vectors.iter().enumerate() {
        let hash_bytes = hex::decode(&gv.expected_hash[2..]).unwrap();
        assert!(hash_bytes.iter().any(|&b| b != 0), "Golden vector {}: hash is all zeros", i);
        let sig_bytes = hex::decode(&gv.signature[2..]).unwrap();
        assert_eq!(sig_bytes.len(), 64, "Golden vector {}: sig should be 64 bytes", i);
    }

    // Vectors 5 (with metadata) and 6 (without) must have different hashes
    assert_ne!(
        vectors[4].expected_hash, vectors[5].expected_hash,
        "metadata vs no-metadata should produce different hashes"
    );

    // Vectors 1 (transfer to self) and 7 (transfer to pk2) must have different hashes
    assert_ne!(
        vectors[0].expected_hash, vectors[6].expected_hash,
        "different recipients should produce different hashes"
    );
}
