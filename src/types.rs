ALL CODE MUST BE PRODUCTION GRADE AND ABSOLUTELY NO STUBS WITH OUT ASKING!!
use serde::{Deserialize, Serialize};
use ed25519_dalek::{Signature, PublicKey, Verifier};
use sha2::{Sha256, Digest};

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Block {
    pub index: u64,
    pub timestamp: u64,
    pub prev_hash: String,
    pub hash: String,
    pub nonce: u64,
    pub transactions: Vec<Transaction>,
    pub metadata: Option<std::collections::BTreeMap<String, String>>, // Using BTreeMap for deterministic serialization
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub hash: String,
    pub sender: PublicKey,
    pub nonce: u64,
    pub timestamp: u64,
    pub recipient: Address,
    pub payload: TransactionPayload,
    pub signature: Signature,
    pub gas_limit: u64,
    pub priority: u8,
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Address {
    Wallet(PublicKey),
    Contract(ContractId),
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct ContractId {
    pub id: String, // Hex-encoded hash of the contract
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
    DeployContract {
        wasm_bytes: Vec<u8>,
        init_payload: Option<Vec<u8>>,
    },
    CallContract {
        method_name: String,
        args: Vec<Vec<u8>>, // Serialized arguments
    },
    Data {
        data: Vec<u8>,
    },
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct ChainState {
    pub latest_block_hash: String,
    pub latest_block_index: u64,
    pub accounts_root_hash: String, // Merkle root of the accounts/contract state tree
    pub total_supply: u64, // (Optional) If BaaLS has a native token
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Account {
    Wallet {
        balance: u64,
        nonce: u64,
    },
    Contract {
        code_hash: String, // Hash of the deployed WASM module
        storage_root_hash: String, // Merkle root of the contract's internal key-value storage
        nonce: u64,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Invalid signature")]
    InvalidSignature(#[from] ed25519_dalek::SignatureError),
    #[error("Invalid public key")]
    InvalidPublicKey(#[from] ed25519_dalek::ed25519::Error),
    #[error("Hashing failed")]
    HashingFailed,
}

impl Block {
    pub fn calculate_hash(&self) -> Result<String, CryptoError> {
        let mut hasher = Sha256::new();
        hasher.update(self.index.to_le_bytes());
        hasher.update(self.timestamp.to_le_bytes());
        hasher.update(&self.prev_hash);
        hasher.update(self.nonce.to_le_bytes());

        // Serialize transactions deterministically
        let serialized_txns = bincode::serialize(&self.transactions)
            .map_err(|_| CryptoError::HashingFailed)?;
        hasher.update(serialized_txns);

        // Serialize metadata deterministically
        if let Some(metadata) = &self.metadata {
            let serialized_metadata = bincode::serialize(metadata)
                .map_err(|_| CryptoError::HashingFailed)?;
            hasher.update(serialized_metadata);
        }

        Ok(format!("{:x}", hasher.finalize()))
    }
}

impl Transaction {
    pub fn calculate_hash(&self) -> Result<String, CryptoError> {
        let mut hasher = Sha256::new();
        hasher.update(self.sender.as_bytes());
        hasher.update(self.nonce.to_le_bytes());
        hasher.update(self.timestamp.to_le_bytes());

        // Serialize recipient deterministically
        let serialized_recipient = bincode::serialize(&self.recipient)
            .map_err(|_| CryptoError::HashingFailed)?;
        hasher.update(serialized_recipient);

        // Serialize payload deterministically
        let serialized_payload = bincode::serialize(&self.payload)
            .map_err(|_| CryptoError::HashingFailed)?;
        hasher.update(serialized_payload);

        // Serialize metadata deterministically
        if let Some(metadata) = &self.metadata {
            let serialized_metadata = bincode::serialize(metadata)
                .map_err(|_| CryptoError::HashingFailed)?;
            hasher.update(serialized_metadata);
        }
        
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn sign(&mut self, private_key: &ed25519_dalek::SigningKey) -> Result<(), CryptoError> {
        self.hash = self.calculate_hash()?; // Calculate hash first
        let signature = private_key.sign(self.hash.as_bytes());
        self.signature = signature;
        Ok(())
    }

    pub fn verify_signature(&self) -> Result<bool, CryptoError> {
        let public_key: PublicKey = self.sender; // Clone the public key
        let expected_hash = self.calculate_hash()?; // Recalculate hash for verification

        if self.hash != expected_hash {
            return Ok(false); // Hash mismatch
        }

        Ok(public_key.verify(self.hash.as_bytes(), &self.signature).is_ok())
    }
} 