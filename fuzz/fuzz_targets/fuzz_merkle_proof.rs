#![no_main]

use baals::{MerkleTree, SparseMerkleProof, SparseMerkleTree};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Test Merkle proof deserialization and verification
    if let Ok(proof) = bincode::deserialize::<SparseMerkleProof>(data) {
        let _ = SparseMerkleTree::verify_proof(proof.key, &proof.value, &proof, proof.root);
        // Also try with tampered key
        let mut tampered_key = proof.key;
        tampered_key[0] = tampered_key[0].wrapping_add(1);
        let _ = SparseMerkleTree::verify_proof(tampered_key, &proof.value, &proof, proof.root);
        // Try with empty value
        let _ = SparseMerkleTree::verify_proof(proof.key, &[], &proof, proof.root);
    }

    // Test binary Merkle tree with arbitrary data
    let mut tree = MerkleTree::new();
    for chunk in data.chunks(32) {
        tree.add_leaf_hash({
            let mut leaf = [0u8; 32];
            let len = chunk.len().min(32);
            leaf[..len].copy_from_slice(&chunk[..len]);
            leaf
        });
    }
    if let Ok(root) = tree.root() {
        for i in 0..data.chunks(32).len().min(16) {
            if let Ok(proof) = tree.generate_proof(i) {
                let leaf = if i * 32 + 32 <= data.len() {
                    let mut l = [0u8; 32];
                    l.copy_from_slice(&data[i * 32..i * 32 + 32]);
                    l
                } else {
                    [0u8; 32]
                };
                let _ = tree.verify_proof(i, &leaf, &proof, root);
                // Tampered leaf should fail
                let mut bad_leaf = leaf;
                bad_leaf[0] = bad_leaf[0].wrapping_add(1);
                let _ = tree.verify_proof(i, &bad_leaf, &proof, root);
            }
        }
    }
});
