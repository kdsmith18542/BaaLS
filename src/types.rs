use ed25519_dalek::{Signature, SignatureError, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Hash conversion error")]
    HashConversionError,
    #[error("Signature verification failed")]
    SignatureVerificationFailed,
    #[error("Invalid public key")]
    InvalidPublicKey,
    #[error("Invalid signature")]
    InvalidSignature,
}

#[derive(Debug, Error)]
pub enum MerkleError {
    #[error("Invalid leaf index")]
    InvalidLeafIndex,
    #[error("Invalid proof")]
    InvalidProof,
    #[error("Empty tree")]
    EmptyTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
    Blake3,
}

pub fn hash_with_algorithm(data: &[u8], algorithm: HashAlgorithm) -> [u8; 32] {
    match algorithm {
        HashAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            hasher.update(data);
            hasher.finalize().into()
        }
        HashAlgorithm::Blake3 => {
            let mut hasher = blake3::Hasher::new();
            hasher.update(data);
            hasher.finalize().into()
        }
    }
}

/// Simple Merkle Tree implementation for state verification
pub struct MerkleTree {
    leaves: Vec<[u8; 32]>,
    root: RefCell<Option<[u8; 32]>>,
    algorithm: HashAlgorithm,
}

impl MerkleTree {
    pub fn new() -> Self {
        Self {
            leaves: Vec::new(),
            root: RefCell::new(None),
            algorithm: HashAlgorithm::Sha256,
        }
    }

    pub fn with_algorithm(algorithm: HashAlgorithm) -> Self {
        Self {
            leaves: Vec::new(),
            root: RefCell::new(None),
            algorithm,
        }
    }

    pub fn add_leaf(&mut self, data: &[u8]) {
        let hash = self.hash_data(data);
        self.leaves.push(hash);
        *self.root.borrow_mut() = None; // Invalidate cached root
    }

    pub fn add_leaf_hash(&mut self, hash: [u8; 32]) {
        self.leaves.push(hash);
        *self.root.borrow_mut() = None; // Invalidate cached root
    }

    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub fn root(&self) -> Result<[u8; 32], MerkleError> {
        if self.leaves.is_empty() {
            return Err(MerkleError::EmptyTree);
        }

        if let Some(root) = *self.root.borrow() {
            return Ok(root);
        }

        let mut current_level = self.leaves.clone();
        let algo = self.algorithm;

        while current_level.len() > 1 {
            let mut next_level = Vec::new();

            for chunk in current_level.chunks(2) {
                let mut combined = Vec::from(chunk[0]);
                if chunk.len() == 2 {
                    combined.extend_from_slice(&chunk[1]);
                } else {
                    combined.extend_from_slice(&chunk[0]);
                }
                let hash = hash_with_algorithm(&combined, algo);
                next_level.push(hash);
            }

            current_level = next_level;
        }

        *self.root.borrow_mut() = Some(current_level[0]);
        Ok(current_level[0])
    }

    pub fn generate_proof(&self, leaf_index: usize) -> Result<Vec<[u8; 32]>, MerkleError> {
        if leaf_index >= self.leaves.len() {
            return Err(MerkleError::InvalidLeafIndex);
        }

        let mut proof = Vec::new();
        let mut current_index = leaf_index;
        let mut current_level = self.leaves.clone();
        let algo = self.algorithm;

        while current_level.len() > 1 {
            let sibling_index = if current_index % 2 == 0 {
                current_index + 1
            } else {
                current_index - 1
            };

            if sibling_index < current_level.len() {
                proof.push(current_level[sibling_index]);
            } else {
                proof.push(current_level[current_index]);
            }

            current_index /= 2;
            let mut next_level = Vec::new();

            for chunk in current_level.chunks(2) {
                let mut combined = Vec::from(chunk[0]);
                if chunk.len() == 2 {
                    combined.extend_from_slice(&chunk[1]);
                } else {
                    combined.extend_from_slice(&chunk[0]);
                }
                let hash = hash_with_algorithm(&combined, algo);
                next_level.push(hash);
            }

            current_level = next_level;
        }

        Ok(proof)
    }

    pub fn verify_proof(
        &self,
        leaf_index: usize,
        leaf_data: &[u8],
        proof: &[[u8; 32]],
        root: [u8; 32],
    ) -> Result<bool, MerkleError> {
        if leaf_index >= self.leaves.len() {
            return Err(MerkleError::InvalidLeafIndex);
        }

        let leaf_hash = self.hash_data(leaf_data);
        let mut current_hash = leaf_hash;
        let mut current_index = leaf_index;
        let algo = self.algorithm;

        for proof_element in proof.iter() {
            let combined: Vec<u8> = if current_index % 2 == 0 {
                let mut c = Vec::from(current_hash);
                c.extend_from_slice(proof_element);
                c
            } else {
                let mut c = Vec::from(*proof_element);
                c.extend_from_slice(&current_hash);
                c
            };
            current_hash = hash_with_algorithm(&combined, algo);
            current_index /= 2;
        }

        Ok(current_hash == root)
    }

    fn hash_data(&self, data: &[u8]) -> [u8; 32] {
        hash_with_algorithm(data, self.algorithm)
    }
}

// Remove serde derive from PublicKey since VerifyingKey doesn't support it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, CryptoError> {
        VerifyingKey::from_bytes(bytes)
            .map(PublicKey)
            .map_err(|_| CryptoError::InvalidPublicKey)
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), SignatureError> {
        self.0.verify(message, signature)
    }
}

impl From<VerifyingKey> for PublicKey {
    fn from(vk: VerifyingKey) -> Self {
        PublicKey(vk)
    }
}

impl From<PublicKey> for VerifyingKey {
    fn from(pk: PublicKey) -> Self {
        pk.0
    }
}

// Manual serde implementation for PublicKey
impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_bytes().as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("Invalid public key length"));
        }
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("Invalid public key length"))?;
        PublicKey::from_bytes(&arr).map_err(serde::de::Error::custom)
    }
}

// Remove serde derive from Signature since ed25519_dalek::Signature doesn't support it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionSignature(ed25519_dalek::Signature);

impl TransactionSignature {
    pub fn from_bytes(bytes: &[u8; 64]) -> Result<Self, CryptoError> {
        Ok(TransactionSignature(ed25519_dalek::Signature::from_bytes(
            bytes,
        )))
    }

    pub fn to_bytes(&self) -> [u8; 64] {
        self.0.to_bytes()
    }
}

impl From<ed25519_dalek::Signature> for TransactionSignature {
    fn from(sig: ed25519_dalek::Signature) -> Self {
        TransactionSignature(sig)
    }
}

impl From<TransactionSignature> for ed25519_dalek::Signature {
    fn from(ts: TransactionSignature) -> Self {
        ts.0
    }
}

// serde: use Vec<u8> semantics (length prefix + bytes) for bincode compatibility
impl Serialize for TransactionSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_bytes().as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TransactionSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::custom(
                "Invalid signature length: expected 64",
            ));
        }
        let arr: [u8; 64] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("Invalid signature length"))?;
        TransactionSignature::from_bytes(&arr).map_err(serde::de::Error::custom)
    }
}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.0.to_bytes().cmp(&other.0.to_bytes()))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.to_bytes().cmp(&other.0.to_bytes())
    }
}

// Helper function for hex formatting
pub fn format_hex(bytes: &[u8; 32]) -> String {
    hex::encode(bytes)
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Block {
    pub index: u64,
    pub timestamp: u64,
    pub prev_hash: [u8; 32],
    pub hash: [u8; 32],
    pub nonce: u64,
    pub transactions: Vec<Transaction>,
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub hash: [u8; 32],
    pub sender: PublicKey,
    pub nonce: u64,
    pub timestamp: u64,
    pub recipient: Address,
    pub payload: TransactionPayload,
    pub signature: TransactionSignature,
    pub gas_limit: u64,
    pub priority: u8,
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
}

impl Transaction {
    pub fn payload_size_estimate(&self) -> usize {
        match &self.payload {
            TransactionPayload::Transfer { .. } => 40,
            TransactionPayload::ContractDeploy { wasm_bytes } => wasm_bytes.len() + 100,
            TransactionPayload::ContractCall { method, args, .. } => method.len() + args.len() + 50,
            TransactionPayload::Data { data } => data.len() + 10,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Address {
    Wallet(PublicKey),
    Contract(ContractId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct ContractId {
    pub id: [u8; 32],
}

impl ContractId {
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        ContractId { id: *bytes }
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.id
    }
}

impl From<PublicKey> for Address {
    fn from(pk: PublicKey) -> Self {
        Address::Wallet(pk)
    }
}

impl From<ContractId> for Address {
    fn from(contract_id: ContractId) -> Self {
        Address::Contract(contract_id)
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Address::Wallet(pk) => write!(f, "Wallet({})", format_hex(&pk.to_bytes())),
            Address::Contract(cid) => write!(f, "Contract({})", format_hex(&cid.id)),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum TransactionPayload {
    Transfer { amount: u64 },
    ContractDeploy { wasm_bytes: Vec<u8> },
    ContractCall {
        method: String,
        args: Vec<u8>,
        value: Option<u64>,
    },
    Data { data: Vec<u8> },
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct ChainState {
    pub latest_block_hash: [u8; 32],
    pub latest_block_index: u64,
    pub accounts_root_hash: [u8; 32], // Merkle root of the accounts/contract state tree
    pub total_supply: u64,            // (Optional) If BaaLS has a native token
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Account {
    Wallet {
        balance: u64,
        nonce: u64,
    },
    Contract {
        code_hash: [u8; 32],         // Hash of the deployed WASM module
        storage_root_hash: [u8; 32], // Merkle root of the contract's internal key-value storage
        nonce: u64,
    },
}

impl Account {
    pub fn nonce(&self) -> u64 {
        match self {
            Account::Wallet { nonce, .. } => *nonce,
            Account::Contract { nonce, .. } => *nonce,
        }
    }

    pub fn set_nonce(&mut self, new_nonce: u64) {
        match self {
            Account::Wallet { nonce, .. } => *nonce = new_nonce,
            Account::Contract { nonce, .. } => *nonce = new_nonce,
        }
    }
}

impl Block {
    pub fn calculate_hash(&self) -> Result<[u8; 32], CryptoError> {
        let mut hasher = Sha256::new();
        hasher.update(self.index.to_le_bytes());
        hasher.update(self.timestamp.to_le_bytes());
        hasher.update(self.prev_hash);
        hasher.update(self.nonce.to_le_bytes());

        // Serialize transactions deterministically
        let serialized_txns =
            bincode::serialize(&self.transactions).map_err(|_| CryptoError::HashConversionError)?;
        hasher.update(serialized_txns);

        Ok(hasher.finalize().into())
    }
}

impl Transaction {
    pub fn calculate_hash(&self) -> Result<[u8; 32], CryptoError> {
        let mut hasher = Sha256::new();
        hasher.update(self.sender.as_bytes());
        hasher.update(self.nonce.to_le_bytes());
        hasher.update(self.timestamp.to_le_bytes());
        hasher.update(self.gas_limit.to_le_bytes());
        hasher.update(self.priority.to_le_bytes());

        // Serialize recipient deterministically
        let serialized_recipient =
            bincode::serialize(&self.recipient).map_err(|_| CryptoError::HashConversionError)?;
        hasher.update(serialized_recipient);

        // Serialize payload deterministically
        let serialized_payload =
            bincode::serialize(&self.payload).map_err(|_| CryptoError::HashConversionError)?;
        hasher.update(serialized_payload);

        // Serialize metadata deterministically
        if let Some(metadata) = &self.metadata {
            let serialized_metadata =
                bincode::serialize(metadata).map_err(|_| CryptoError::HashConversionError)?;
            hasher.update(serialized_metadata);
        }

        Ok(hasher.finalize().into())
    }

    pub fn sign(&mut self, private_key: &SigningKey) -> Result<(), CryptoError> {
        self.hash = self.calculate_hash()?; // Calculate hash first
        let signature = private_key.sign(&self.hash);
        self.signature = TransactionSignature::from(signature);
        Ok(())
    }

    pub fn verify_signature(&self) -> Result<bool, CryptoError> {
        let public_key: PublicKey = self.sender; // Clone the public key
        let expected_hash = self.calculate_hash()?; // Recalculate hash for verification

        if self.hash != expected_hash {
            return Ok(false); // Hash mismatch
        }

        Ok(public_key.verify(&self.hash, &self.signature.0).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;
    use rand::RngCore;

    #[test]
    fn test_block_hash_calculation() {
        let mut rng = OsRng;
        let mut sk_bytes = [0u8; 32];
        rng.fill_bytes(&mut sk_bytes);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sk_bytes);
        let sender_pk = PublicKey::from(signing_key.verifying_key());

        let tx1 = Transaction {
            hash: [0; 32],
            sender: sender_pk,
            nonce: 1,
            timestamp: 1234567890,
            recipient: Address::Wallet(sender_pk),
            payload: TransactionPayload::Data {
                data: vec![1, 2, 3],
            },
            signature: TransactionSignature::from_bytes(&[0; 64]).unwrap(),
            gas_limit: 0,
            priority: 0,
            metadata: None,
        };
        let tx2 = Transaction {
            hash: [0; 32],
            sender: sender_pk,
            nonce: 2,
            timestamp: 1234567891,
            recipient: Address::Wallet(sender_pk),
            payload: TransactionPayload::Data {
                data: vec![4, 5, 6],
            },
            signature: TransactionSignature::from_bytes(&[0; 64]).unwrap(),
            gas_limit: 0,
            priority: 0,
            metadata: None,
        };

        let block = Block {
            index: 0,
            timestamp: 1234567890,
            prev_hash: [0; 32],
            hash: [0; 32],
            nonce: 0,
            transactions: vec![tx1.clone(), tx2.clone()],
            metadata: None,
        };

        let hash1 = block.calculate_hash().unwrap();
        // Changing a field should change the hash
        let mut block_modified = block.clone();
        block_modified.index = 1;
        let hash2 = block_modified.calculate_hash().unwrap();
        assert_ne!(hash1, hash2);

        // Same content should yield same hash
        let block_copy = block.clone();
        let hash3 = block_copy.calculate_hash().unwrap();
        assert_eq!(hash1, hash3);
    }

    #[test]
    fn test_transaction_signing_and_verification() {
        let mut rng = OsRng;
        let mut sk_bytes = [0u8; 32];
        rng.fill_bytes(&mut sk_bytes);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sk_bytes);
        let public_key = PublicKey::from(signing_key.verifying_key());
        let private_key = signing_key;

        let mut tx = Transaction {
            hash: [0; 32],
            sender: public_key,
            nonce: 1,
            timestamp: 1234567890,
            recipient: Address::Wallet(public_key),
            payload: TransactionPayload::Data {
                data: vec![1, 2, 3],
            },
            signature: TransactionSignature::from_bytes(&[0; 64]).unwrap(),
            gas_limit: 0,
            priority: 0,
            metadata: None,
        };

        // Before signing, hash is default and verification should fail
        assert!(!tx.verify_signature().unwrap());

        // Sign the transaction
        tx.sign(&private_key).unwrap();
        assert_ne!(tx.hash, [0; 32]); // Hash should be calculated

        // After signing, verification should pass
        assert!(tx.verify_signature().unwrap());

        // Tampering with payload should make verification fail
        let mut tampered_tx = tx.clone();
        if let TransactionPayload::Data { data } = &mut tampered_tx.payload {
            data.push(99);
        }
        // Recalculate hash for tampering, but don't re-sign
        tampered_tx.hash = tampered_tx.calculate_hash().unwrap();
        assert!(!tampered_tx.verify_signature().unwrap());

        // Tampering with signature should make verification fail
        let mut tampered_sig_tx = tx.clone();
        tampered_sig_tx.signature = TransactionSignature::from_bytes(&[1; 64]).unwrap(); // Invalid signature
        assert!(!tampered_sig_tx.verify_signature().unwrap());
    }

    #[test]
    fn test_merkle_tree_single_leaf() {
        let mut tree = MerkleTree::new();
        tree.add_leaf(b"hello");
        let root = tree.root().unwrap();
        let proof = tree.generate_proof(0).unwrap();
        assert!(proof.is_empty());
        assert!(tree.verify_proof(0, b"hello", &proof, root).unwrap());
    }

    #[test]
    fn test_merkle_tree_two_leaves() {
        let mut tree = MerkleTree::new();
        tree.add_leaf(b"left");
        tree.add_leaf(b"right");
        let root = tree.root().unwrap();

        // Verify left leaf (index 0)
        let proof_left = tree.generate_proof(0).unwrap();
        assert_eq!(proof_left.len(), 1);
        assert!(tree.verify_proof(0, b"left", &proof_left, root).unwrap());

        // Verify right leaf (index 1)
        let proof_right = tree.generate_proof(1).unwrap();
        assert_eq!(proof_right.len(), 1);
        assert!(tree.verify_proof(1, b"right", &proof_right, root).unwrap());

        // Wrong leaf data should fail
        assert!(!tree.verify_proof(0, b"wrong", &proof_left, root).unwrap());
    }

    #[test]
    fn test_merkle_tree_three_leaves() {
        let mut tree = MerkleTree::new();
        tree.add_leaf(b"a");
        tree.add_leaf(b"b");
        tree.add_leaf(b"c");
        let root = tree.root().unwrap();

        // Verify each leaf
        for (i, data) in [b"a", b"b", b"c"].iter().enumerate() {
            let proof = tree.generate_proof(i).unwrap();
            assert!(tree.verify_proof(i, *data, &proof, root).unwrap());
        }

        // Wrong index should fail (proof for index 0 with index 1)
        let proof_0 = tree.generate_proof(0).unwrap();
        assert!(!tree.verify_proof(1, b"a", &proof_0, root).unwrap());

        // With 3 leaves and duplication, tree height is 2
        let proof_c = tree.generate_proof(2).unwrap();
        assert_eq!(proof_c.len(), 2);
        assert!(tree.verify_proof(2, b"c", &proof_c, root).unwrap());
    }

    #[test]
    fn test_merkle_tree_four_leaves() {
        let mut tree = MerkleTree::new();
        tree.add_leaf(b"leaf0");
        tree.add_leaf(b"leaf1");
        tree.add_leaf(b"leaf2");
        tree.add_leaf(b"leaf3");
        let root = tree.root().unwrap();

        // Verify each leaf
        for i in 0..4 {
            let data = format!("leaf{}", i);
            let proof = tree.generate_proof(i).unwrap();
            assert!(tree.verify_proof(i, data.as_bytes(), &proof, root).unwrap());
        }
    }

    #[test]
    fn test_merkle_tree_tampered_proof_fails() {
        let mut tree = MerkleTree::new();
        tree.add_leaf(b"alpha");
        tree.add_leaf(b"beta");
        tree.add_leaf(b"gamma");
        tree.add_leaf(b"delta");
        let root = tree.root().unwrap();

        let proof = tree.generate_proof(0).unwrap();
        assert!(tree.verify_proof(0, b"alpha", &proof, root).unwrap());

        // Tamper with a proof element
        let mut tampered_proof = proof.clone();
        tampered_proof[0] = [0xff; 32];
        assert!(!tree
            .verify_proof(0, b"alpha", &tampered_proof, root)
            .unwrap());
    }

    #[test]
    fn test_merkle_tree_wrong_root_fails() {
        let mut tree = MerkleTree::new();
        tree.add_leaf(b"x");
        tree.add_leaf(b"y");
        let root = tree.root().unwrap();

        let mut other_tree = MerkleTree::new();
        other_tree.add_leaf(b"a");
        other_tree.add_leaf(b"b");
        let other_root = other_tree.root().unwrap();

        let proof = tree.generate_proof(0).unwrap();
        assert!(tree.verify_proof(0, b"x", &proof, root).unwrap());
        assert!(!tree.verify_proof(0, b"x", &proof, other_root).unwrap());
    }

    #[test]
    fn test_merkle_tree_invalid_leaf_index() {
        let mut tree = MerkleTree::new();
        tree.add_leaf(b"only");
        assert!(matches!(
            tree.generate_proof(1),
            Err(MerkleError::InvalidLeafIndex)
        ));
        assert!(matches!(
            tree.verify_proof(1, b"only", &[], [0; 32]),
            Err(MerkleError::InvalidLeafIndex)
        ));
    }

    #[test]
    fn test_merkle_tree_empty() {
        let mut tree = MerkleTree::new();
        assert!(matches!(tree.root(), Err(MerkleError::EmptyTree)));
    }

    #[test]
    fn test_merkle_tree_consistency() {
        let mut tree = MerkleTree::new();
        tree.add_leaf(b"data1");
        tree.add_leaf(b"data2");
        let root1 = tree.root().unwrap();
        let root2 = tree.root().unwrap();
        assert_eq!(root1, root2);

        // Adding another leaf invalidates cached root
        tree.add_leaf(b"data3");
        let root3 = tree.root().unwrap();
        assert_ne!(root1, root3);
    }
}
