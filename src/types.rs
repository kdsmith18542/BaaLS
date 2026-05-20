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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HashAlgorithm {
    #[default]
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
#[derive(Default)]
pub struct MerkleTree {
    leaves: Vec<[u8; 32]>,
    root: RefCell<Option<[u8; 32]>>,
    algorithm: HashAlgorithm,
}

impl MerkleTree {
    pub fn new() -> Self {
        Self { leaves: Vec::new(), root: RefCell::new(None), algorithm: HashAlgorithm::Sha256 }
    }

    pub fn with_algorithm(algorithm: HashAlgorithm) -> Self {
        Self { leaves: Vec::new(), root: RefCell::new(None), algorithm }
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
            let sibling_index =
                if current_index.is_multiple_of(2) { current_index + 1 } else { current_index - 1 };

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
            let combined: Vec<u8> = if current_index.is_multiple_of(2) {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparseMerkleProof {
    pub key: [u8; 32],
    pub value: Vec<u8>,
    pub proof: Vec<[u8; 32]>,
    pub root: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageUpdateSet {
    pub writes: std::collections::HashMap<Vec<u8>, Vec<u8>>,
    pub deletes: Vec<Vec<u8>>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ContractExecutionSideEffects {
    pub storage_updates: StorageUpdateSet,
    pub events: Vec<(Vec<u8>, Vec<u8>)>,
}

/// Key-indexed sparse Merkle tree (256-bit keys).
/// Leaves are hashed as H(0x00 || key || H(value)), and internal nodes as H(0x01 || left || right).
#[derive(Default)]
pub struct SparseMerkleTree {
    leaves: std::collections::BTreeMap<[u8; 32], Vec<u8>>,
}

type BuildLevelsResult = (Vec<std::collections::BTreeMap<[u8; 32], [u8; 32]>>, Vec<[u8; 32]>);

impl SparseMerkleTree {
    pub fn new() -> Self {
        Self { leaves: std::collections::BTreeMap::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub fn insert(&mut self, key: [u8; 32], value: Vec<u8>) {
        self.leaves.insert(key, value);
    }

    pub fn root(&self) -> [u8; 32] {
        let (levels, defaults) = self.build_levels();
        levels[0].get(&[0u8; 32]).copied().unwrap_or(defaults[0])
    }

    pub fn generate_proof(&self, key: [u8; 32]) -> SparseMerkleProof {
        let (levels, defaults) = self.build_levels();
        let mut siblings = Vec::with_capacity(256);
        let mut prefix = key;

        for depth in (1..=256).rev() {
            let bit = Self::get_bit(&key, depth - 1);
            let sibling_prefix = {
                let mut s = Self::truncate_key(prefix, depth);
                Self::set_bit(&mut s, depth - 1, if bit == 0 { 1 } else { 0 });
                Self::truncate_key(s, depth)
            };
            let sibling_hash =
                levels[depth].get(&sibling_prefix).copied().unwrap_or(defaults[depth]);
            siblings.push(sibling_hash);
            prefix = Self::truncate_key(prefix, depth - 1);
        }

        let value = self.leaves.get(&key).cloned().unwrap_or_default();
        SparseMerkleProof {
            key,
            value,
            proof: siblings,
            root: levels[0].get(&[0u8; 32]).copied().unwrap_or(defaults[0]),
        }
    }

    pub fn verify_proof(
        key: [u8; 32],
        value: &[u8],
        proof: &SparseMerkleProof,
        expected_root: [u8; 32],
    ) -> bool {
        if proof.proof.len() != 256 {
            return false;
        }

        let mut current = Self::hash_leaf(key, value);
        for (idx, sibling) in proof.proof.iter().enumerate() {
            let depth = 256 - idx;
            let bit = Self::get_bit(&key, depth - 1);
            current = if bit == 0 {
                Self::hash_internal(current, *sibling)
            } else {
                Self::hash_internal(*sibling, current)
            };
        }
        current == expected_root
    }

    pub fn update_with_siblings(
        &mut self,
        key: [u8; 32],
        value: &[u8],
        siblings: &[[u8; 32]; 256],
    ) -> [u8; 32] {
        let mut current = Self::hash_leaf(key, value);
        for (idx, sibling) in siblings.iter().enumerate() {
            let depth = 256 - idx;
            let bit = Self::get_bit(&key, depth - 1);
            current = if bit == 0 {
                Self::hash_internal(current, *sibling)
            } else {
                Self::hash_internal(*sibling, current)
            };
        }
        current
    }

    pub fn get_path_siblings<F>(key: [u8; 32], resolver: F) -> [[u8; 32]; 256]
    where
        F: Fn(u16, [u8; 32]) -> [u8; 32],
    {
        let mut siblings = [[0u8; 32]; 256];
        let mut prefix = key;
        for depth in (1..=256).rev() {
            let bit = Self::get_bit(&key, depth - 1);
            let sibling_prefix = {
                let mut s = Self::truncate_key(prefix, depth);
                Self::set_bit(&mut s, depth - 1, if bit == 0 { 1 } else { 0 });
                Self::truncate_key(s, depth)
            };
            siblings[256 - depth] = resolver(depth as u16, sibling_prefix);
            prefix = Self::truncate_key(prefix, depth - 1);
        }
        siblings
    }

    fn build_levels(&self) -> BuildLevelsResult {
        let defaults = Self::default_hashes();
        let mut levels: Vec<std::collections::BTreeMap<[u8; 32], [u8; 32]>> =
            (0..=256).map(|_| std::collections::BTreeMap::new()).collect();

        for (key, value) in &self.leaves {
            levels[256].insert(*key, Self::hash_leaf(*key, value));
        }

        for depth in (1..=256).rev() {
            let mut parent_keys = std::collections::BTreeSet::new();
            for child_key in levels[depth].keys() {
                parent_keys.insert(Self::truncate_key(*child_key, depth - 1));
            }

            let mut parents = std::collections::BTreeMap::new();
            for parent_key in parent_keys {
                let left_prefix = Self::child_prefix(parent_key, depth, 0);
                let right_prefix = Self::child_prefix(parent_key, depth, 1);
                let left_hash = levels[depth].get(&left_prefix).copied().unwrap_or(defaults[depth]);
                let right_hash =
                    levels[depth].get(&right_prefix).copied().unwrap_or(defaults[depth]);
                parents.insert(parent_key, Self::hash_internal(left_hash, right_hash));
            }
            levels[depth - 1] = parents;
        }

        (levels, defaults)
    }

    pub fn default_hashes() -> Vec<[u8; 32]> {
        let mut defaults = vec![[0u8; 32]; 257];
        defaults[256] = Self::hash_leaf([0u8; 32], &[]);
        for depth in (0..256).rev() {
            defaults[depth] = Self::hash_internal(defaults[depth + 1], defaults[depth + 1]);
        }
        defaults
    }

    pub fn hash_leaf(key: [u8; 32], value: &[u8]) -> [u8; 32] {
        let value_hash: [u8; 32] = Sha256::digest(value).into();
        let mut hasher = Sha256::new();
        hasher.update([0u8]);
        hasher.update(key);
        hasher.update(value_hash);
        hasher.finalize().into()
    }

    pub fn hash_internal(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update([1u8]);
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().into()
    }

    pub fn get_bit(key: &[u8; 32], bit_index: usize) -> u8 {
        let byte_index = bit_index / 8;
        let bit_in_byte = 7 - (bit_index % 8);
        (key[byte_index] >> bit_in_byte) & 1
    }

    fn set_bit(key: &mut [u8; 32], bit_index: usize, value: u8) {
        let byte_index = bit_index / 8;
        let bit_in_byte = 7 - (bit_index % 8);
        if value == 0 {
            key[byte_index] &= !(1 << bit_in_byte);
        } else {
            key[byte_index] |= 1 << bit_in_byte;
        }
    }

    pub fn truncate_key(mut key: [u8; 32], depth_bits: usize) -> [u8; 32] {
        if depth_bits >= 256 {
            return key;
        }
        for bit in depth_bits..256 {
            Self::set_bit(&mut key, bit, 0);
        }
        key
    }

    fn child_prefix(parent_key: [u8; 32], child_depth: usize, bit: u8) -> [u8; 32] {
        let mut key = Self::truncate_key(parent_key, child_depth);
        Self::set_bit(&mut key, child_depth - 1, bit);
        Self::truncate_key(key, child_depth)
    }
}

// Remove serde derive from PublicKey since VerifyingKey doesn't support it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, CryptoError> {
        VerifyingKey::from_bytes(bytes).map(PublicKey).map_err(|_| CryptoError::InvalidPublicKey)
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
        let arr: [u8; 32] =
            bytes.try_into().map_err(|_| serde::de::Error::custom("Invalid public key length"))?;
        PublicKey::from_bytes(&arr).map_err(serde::de::Error::custom)
    }
}

// Remove serde derive from Signature since ed25519_dalek::Signature doesn't support it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionSignature(ed25519_dalek::Signature);

impl TransactionSignature {
    pub fn from_bytes(bytes: &[u8; 64]) -> Result<Self, CryptoError> {
        Ok(TransactionSignature(ed25519_dalek::Signature::from_bytes(bytes)))
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
            return Err(serde::de::Error::custom("Invalid signature length: expected 64"));
        }
        let arr: [u8; 64] =
            bytes.try_into().map_err(|_| serde::de::Error::custom("Invalid signature length"))?;
        TransactionSignature::from_bytes(&arr).map_err(serde::de::Error::custom)
    }
}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub index: u64,
    pub timestamp: u64,
    pub prev_hash: [u8; 32],
    pub state_root: [u8; 32],
    pub hash: [u8; 32],
    pub nonce: u64,
    pub transactions: Vec<Transaction>,
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub total_gas_used: u64,
    #[serde(default)]
    pub signer: Option<String>,
    #[serde(default)]
    pub signature: Option<Vec<u8>>,
    #[serde(default)]
    pub quorum_signatures: Vec<(String, Vec<u8>)>,
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
    pub gas_price: u64,
    pub priority: u8,
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default = "default_chain_id")]
    pub chain_id: u64,
}

fn default_chain_id() -> u64 {
    1
}

impl Transaction {
    pub fn payload_size_estimate(&self) -> usize {
        match &self.payload {
            TransactionPayload::Transfer { .. } => 40,
            TransactionPayload::ContractDeploy { wasm_bytes, init_payload } => {
                wasm_bytes.len() + init_payload.as_ref().map(|p| p.len()).unwrap_or(0) + 100
            }
            TransactionPayload::ContractCall { method, args, .. } => {
                method.len() + args.iter().map(|a| a.len()).sum::<usize>() + args.len() * 4 + 50
            }
            TransactionPayload::Data { data } => data.len() + 10,
            TransactionPayload::ValidatorSetChange { added, removed, .. } => {
                (added.len() + removed.len()) * 32 + 20
            }
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

/// Maps a contract ID to a deterministic, signature-safe account key.
///
/// Some contract IDs are not valid Ed25519 points, while account storage keys use `PublicKey`.
/// This helper keeps contract-account addressing deterministic without relying on raw key validity.
pub fn contract_account_public_key(contract_id: &ContractId) -> PublicKey {
    let raw = contract_id.to_bytes();
    if let Ok(pk) = PublicKey::from_bytes(&raw) {
        return pk;
    }

    let mut counter = 0u32;
    loop {
        let mut hasher = Sha256::new();
        hasher.update(b"baals:contract-account:v1");
        hasher.update(raw);
        hasher.update(counter.to_be_bytes());
        let candidate: [u8; 32] = hasher.finalize().into();
        if let Ok(pk) = PublicKey::from_bytes(&candidate) {
            return pk;
        }
        counter = counter.wrapping_add(1);
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
pub enum TransactionStatus {
    Success,
    Failed(String),
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionReceipt {
    pub tx_hash: [u8; 32],
    pub block_hash: [u8; 32],
    pub block_height: u64,
    pub index_in_block: u32,
    pub success: bool,
    pub gas_used: u64,
    pub contract_address: Option<ContractId>,
    pub error_message: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct TransactionFinality {
    #[serde(rename = "final")]
    pub is_final: bool,
    pub confirmations: u64,
    pub required: u64,
    pub block_height: Option<u64>,
    pub status: TransactionStatus,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum TransactionPayload {
    Transfer { amount: u64 },
    ContractDeploy { wasm_bytes: Vec<u8>, init_payload: Option<Vec<u8>> },
    ContractCall { method: String, args: Vec<Vec<u8>>, value: Option<u64> },
    Data { data: Vec<u8> },
    ValidatorSetChange {
        added: Vec<PublicKey>,
        removed: Vec<PublicKey>,
        effective_height: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSet {
    pub signers: Vec<PublicKey>,
    pub quorum: usize,
    pub effective_height: u64,
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
        balance: u64,
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

    pub fn balance(&self) -> u64 {
        match self {
            Account::Wallet { balance, .. } => *balance,
            Account::Contract { balance, .. } => *balance,
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
        hasher.update(self.state_root);
        hasher.update(self.nonce.to_le_bytes());

        // Compute Merkle root of transaction hashes
        let mut merkle = MerkleTree::new();
        for tx in &self.transactions {
            merkle.add_leaf_hash(tx.hash);
        }
        let tx_root = if merkle.is_empty() {
            [0u8; 32]
        } else {
            merkle.root().map_err(|_| CryptoError::HashConversionError)?
        };
        hasher.update(tx_root);

        // Include signer from metadata in hash — ensures hash covers signer identity
        if let Some(metadata) = &self.metadata {
            if let Some(signer_hex) = metadata.get("signer") {
                hasher.update(signer_hex.as_bytes());
            }
        }

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
        hasher.update(self.gas_price.to_le_bytes());
        hasher.update(self.priority.to_le_bytes());
        hasher.update(self.chain_id.to_le_bytes());

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignerRotation {
    pub added: Vec<PublicKey>,
    pub removed: Vec<PublicKey>,
    pub effective_height: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    #[test]
    fn test_block_hash_calculation() {
        let mut sk_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut sk_bytes);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sk_bytes);
        let sender_pk = PublicKey::from(signing_key.verifying_key());

        let tx1 = Transaction {
            hash: [0; 32],
            sender: sender_pk,
            nonce: 1,
            timestamp: 1234567890,
            recipient: Address::Wallet(sender_pk),
            payload: TransactionPayload::Data { data: vec![1, 2, 3] },
            signature: TransactionSignature::from_bytes(&[0; 64]).unwrap(),
            gas_limit: 21_000,
            gas_price: 1,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let tx2 = Transaction {
            hash: [0; 32],
            sender: sender_pk,
            nonce: 2,
            timestamp: 1234567891,
            recipient: Address::Wallet(sender_pk),
            payload: TransactionPayload::Data { data: vec![4, 5, 6] },
            signature: TransactionSignature::from_bytes(&[0; 64]).unwrap(),
            gas_limit: 21_000,
            gas_price: 1,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };

        let block = Block {
            index: 0,
            timestamp: 1234567890,
            prev_hash: [0; 32],
            state_root: [0; 32],
            hash: [0; 32],
            nonce: 0,
            transactions: vec![tx1.clone(), tx2.clone()],
            metadata: None,
            total_gas_used: 0,
            signer: None,
            signature: None,
            quorum_signatures: Vec::new(),
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
        let mut sk_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut sk_bytes);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sk_bytes);
        let public_key = PublicKey::from(signing_key.verifying_key());
        let private_key = signing_key;

        let mut tx = Transaction {
            hash: [0; 32],
            sender: public_key,
            nonce: 1,
            timestamp: 1234567890,
            recipient: Address::Wallet(public_key),
            payload: TransactionPayload::Data { data: vec![1, 2, 3] },
            signature: TransactionSignature::from_bytes(&[0; 64]).unwrap(),
            gas_limit: 21_000,
            gas_price: 1,
            priority: 0,
            metadata: None,
            chain_id: 1,
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
        assert!(!tree.verify_proof(0, b"alpha", &tampered_proof, root).unwrap());
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
        assert!(matches!(tree.generate_proof(1), Err(MerkleError::InvalidLeafIndex)));
        assert!(matches!(
            tree.verify_proof(1, b"only", &[], [0; 32]),
            Err(MerkleError::InvalidLeafIndex)
        ));
    }

    #[test]
    fn test_merkle_tree_empty() {
        let tree = MerkleTree::new();
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

    #[test]
    fn test_sparse_merkle_proof_roundtrip() {
        let mut tree = SparseMerkleTree::new();
        let key_a = [1u8; 32];
        let key_b = [2u8; 32];
        tree.insert(key_a, b"alpha".to_vec());
        tree.insert(key_b, b"beta".to_vec());

        let root = tree.root();
        let proof_a = tree.generate_proof(key_a);
        assert!(SparseMerkleTree::verify_proof(key_a, b"alpha", &proof_a, root));
        assert!(!SparseMerkleTree::verify_proof(key_a, b"wrong", &proof_a, root));
    }

    #[test]
    fn test_gas_price_affects_hash() {
        let mut sk_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut sk_bytes);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sk_bytes);
        let pk = PublicKey::from(signing_key.verifying_key());

        let mut tx1 = Transaction {
            hash: [0; 32],
            sender: pk,
            nonce: 1,
            timestamp: 1000,
            recipient: Address::Wallet(pk),
            payload: TransactionPayload::Transfer { amount: 100 },
            signature: TransactionSignature::from_bytes(&[0; 64]).unwrap(),
            gas_limit: 100_000,
            gas_price: 0,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let mut tx2 = tx1.clone();
        tx2.gas_price = 10;
        tx1.hash = tx1.calculate_hash().unwrap();
        tx2.hash = tx2.calculate_hash().unwrap();
        assert_ne!(tx1.hash, tx2.hash, "different gas_price should produce different hashes");
    }

    #[test]
    fn test_gas_price_serialization_roundtrip() {
        let mut sk_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut sk_bytes);
        let sk = ed25519_dalek::SigningKey::from_bytes(&sk_bytes);
        let pk = PublicKey::from(sk.verifying_key());
        let tx = Transaction {
            hash: [0; 32],
            sender: pk,
            nonce: 1,
            timestamp: 1000,
            recipient: Address::Wallet(pk),
            payload: TransactionPayload::Data { data: vec![1, 2, 3] },
            signature: TransactionSignature::from_bytes(&[0; 64]).unwrap(),
            gas_limit: 100_000,
            gas_price: 42,
            priority: 0,
            metadata: None,
            chain_id: 1,
        };
        let encoded = bincode::serialize(&tx).unwrap();
        let decoded: Transaction = bincode::deserialize(&encoded).unwrap();
        assert_eq!(decoded.gas_price, 42);
        assert_eq!(decoded.gas_limit, 100_000);

        let json = serde_json::to_string(&tx).unwrap();
        let from_json: Transaction = serde_json::from_str(&json).unwrap();
        assert_eq!(from_json.gas_price, 42);
        assert_eq!(from_json.gas_limit, 100000);
    }
}
