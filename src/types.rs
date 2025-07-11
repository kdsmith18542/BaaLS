use serde::{Deserialize, Serialize};
use ed25519_dalek::{Signature, VerifyingKey, Verifier, SigningKey, SignatureError, Signer};
use sha2::{Sha256, Digest};
use sha2::digest::FixedOutput;
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
        serializer.serialize_bytes(&self.to_bytes())
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = <[u8; 32]>::deserialize(deserializer)?;
        PublicKey::from_bytes(&bytes).map_err(serde::de::Error::custom)
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

// Manual serde implementation for TransactionSignature
impl Serialize for TransactionSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(&self.to_bytes())
    }
}

impl<'de> Deserialize<'de> for TransactionSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::custom("Invalid signature length"));
        }
        let bytes_array: [u8; 64] = bytes.try_into().map_err(|_| serde::de::Error::custom("Invalid signature length"))?;
        TransactionSignature::from_bytes(&bytes_array).map_err(serde::de::Error::custom)
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
    pub metadata: Option<std::collections::BTreeMap<String, String>>, // Using BTreeMap for deterministic serialization
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

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Address {
    Wallet(PublicKey),
    Contract(ContractId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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


#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum TransactionPayload {
    Transfer {
        amount: u64,
    },
    ContractDeploy {
        wasm_bytes: Vec<u8>,
    },
    ContractCall {
        method: String,
        args: Vec<u8>,
    },
    Data {
        data: Vec<u8>,
    },
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct ChainState {
    pub latest_block_hash: [u8; 32],
    pub latest_block_index: u64,
    pub accounts_root_hash: [u8; 32], // Merkle root of the accounts/contract state tree
    pub total_supply: u64, // (Optional) If BaaLS has a native token
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Account {
    Wallet {
        balance: u64,
        nonce: u64,
    },
    Contract {
        code_hash: [u8; 32], // Hash of the deployed WASM module
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
        let serialized_txns = bincode::serialize(&self.transactions)
            .map_err(|_| CryptoError::HashConversionError)?;
        hasher.update(serialized_txns);

        // Serialize metadata deterministically
        if let Some(metadata) = &self.metadata {
            let serialized_metadata = bincode::serialize(metadata)
                .map_err(|_| CryptoError::HashConversionError)?;
            hasher.update(serialized_metadata);
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

        // Serialize recipient deterministically
        let serialized_recipient = bincode::serialize(&self.recipient)
            .map_err(|_| CryptoError::HashConversionError)?;
        hasher.update(serialized_recipient);

        // Serialize payload deterministically
        let serialized_payload = bincode::serialize(&self.payload)
            .map_err(|_| CryptoError::HashConversionError)?;
        hasher.update(serialized_payload);

        // Serialize metadata deterministically
        if let Some(metadata) = &self.metadata {
            let serialized_metadata = bincode::serialize(metadata)
                .map_err(|_| CryptoError::HashConversionError)?;
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
    use ed25519_dalek::Keypair;
    use rand::rngs::OsRng;

    #[test]
    fn test_block_hash_calculation() {
        let keypair = Keypair::generate(&mut OsRng);
        let sender_pk = keypair.public;

        let tx1 = Transaction {
            hash: [0; 32],
            sender: sender_pk,
            nonce: 1,
            timestamp: 1234567890,
            recipient: Address::Wallet(sender_pk),
            payload: TransactionPayload::Data { data: vec![1, 2, 3] },
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
            payload: TransactionPayload::Data { data: vec![4, 5, 6] },
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
        let mut csprng = OsRng;
        let keypair = Keypair::generate(&mut csprng);
        let public_key = keypair.public;
        let private_key = keypair.signing_key;

        let mut tx = Transaction {
            hash: [0; 32],
            sender: public_key,
            nonce: 1,
            timestamp: 1234567890,
            recipient: Address::Wallet(public_key),
            payload: TransactionPayload::Data { data: vec![1, 2, 3] },
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
} 