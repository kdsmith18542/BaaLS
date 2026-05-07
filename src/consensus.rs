use ed25519_dalek::{Signer, SigningKey};
use hex;
use log::info;
use thiserror::Error;

use crate::types::{Block, ChainState, CryptoError, PublicKey, Transaction};

#[derive(Debug, Error)]
pub enum ConsensusError {
    #[error("Block validation failed: {0}")]
    ValidationFailed(String),
    #[error("Invalid signature: {0}")]
    InvalidSignature(#[from] CryptoError),
    #[error("Unauthorized signer")]
    UnauthorizedSigner,
    #[error("Block timestamp is invalid")]
    InvalidTimestamp,
    #[error("Mismatched previous hash")]
    MismatchedPrevHash,
    #[error("Invalid nonce")]
    InvalidNonce,
    #[error("No pending transactions available to generate a block")]
    NoPendingTransactions,
    #[error("Block signing failed: {0}")]
    BlockSigningFailed(String),
}

pub trait ConsensusEngine: Send + Sync {
    fn validate_block(&self, block: &Block, chain_state: &ChainState)
        -> Result<(), ConsensusError>;
    fn generate_block(
        &self,
        pending_transactions: &[Transaction],
        prev_block: &Block,
        chain_state: &ChainState,
    ) -> Result<Block, ConsensusError>;

    fn block_time_interval_ms(&self) -> u64 {
        5000
    }
}

pub struct PoAConsensus {
    authorized_signer_key: PublicKey,
    #[allow(dead_code)]
    block_time_interval_ms: u64,
    signing_key: Option<SigningKey>,
    pub block_gas_limit: u64,
    pub block_size_limit: usize,
}

impl PoAConsensus {
    const MAX_FUTURE_BLOCK_TIMESTAMP_SECONDS: u64 = 10;
    pub fn new(authorized_signer_key: PublicKey, block_time_interval_ms: u64) -> Self {
        Self {
            authorized_signer_key,
            block_time_interval_ms,
            signing_key: None,
            block_gas_limit: 30_000_000,        // 30M gas per block
            block_size_limit: 10 * 1024 * 1024, // 10MB per block
        }
    }

    pub fn with_signing_key(mut self, signing_key: SigningKey) -> Self {
        self.signing_key = Some(signing_key);
        self
    }

    pub fn validate_block(&self, block: &Block) -> Result<(), ConsensusError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| ConsensusError::InvalidTimestamp)?
            .as_secs();
        if block.timestamp > now + Self::MAX_FUTURE_BLOCK_TIMESTAMP_SECONDS {
            return Err(ConsensusError::InvalidTimestamp);
        }

        let metadata = block.metadata.as_ref().ok_or_else(|| {
            ConsensusError::ValidationFailed("Block metadata missing".to_string())
        })?;

        // Verify signer matches authorized key
        let signer_hex = metadata.get("signer").ok_or_else(|| {
            ConsensusError::ValidationFailed("Signer not found in block metadata".to_string())
        })?;
        if *signer_hex != hex::encode(self.authorized_signer_key.to_bytes()) {
            return Err(ConsensusError::UnauthorizedSigner);
        }

        // Verify signature cryptographically
        let signature_hex = metadata.get("signature").ok_or_else(|| {
            ConsensusError::ValidationFailed("Signature not found in block metadata".to_string())
        })?;
        let signature_bytes = hex::decode(signature_hex).map_err(|_| {
            ConsensusError::ValidationFailed("Invalid signature hex encoding".to_string())
        })?;
        if signature_bytes.len() != 64 {
            return Err(ConsensusError::ValidationFailed("Invalid signature length".to_string()));
        }
        let signature = ed25519_dalek::Signature::from_slice(&signature_bytes).map_err(|_| {
            ConsensusError::ValidationFailed("Invalid signature format".to_string())
        })?;

        self.authorized_signer_key.verify(&block.hash, &signature).map_err(|_| {
            ConsensusError::InvalidSignature(CryptoError::SignatureVerificationFailed)
        })?;

        // Nonce check — for PoA, nonce should be 0
        if block.nonce != 0 {
            return Err(ConsensusError::InvalidNonce);
        }

        Ok(())
    }

    pub fn sign_block(&self, block: &mut Block) -> Result<(), ConsensusError> {
        if let Some(signing_key) = &self.signing_key {
            // Verify the private key corresponds to the authorized signer
            if signing_key.verifying_key().to_bytes() != self.authorized_signer_key.to_bytes() {
                return Err(ConsensusError::UnauthorizedSigner);
            }

            // Calculate block hash if not already set
            if block.hash == [0u8; 32] {
                block.hash = block
                    .calculate_hash()
                    .map_err(|e| ConsensusError::BlockSigningFailed(e.to_string()))?;
            }

            // Sign the block hash
            let signature = signing_key.sign(&block.hash);

            // Add signature to block metadata
            let mut metadata = block.metadata.clone().unwrap_or_default();
            metadata
                .insert("signer".to_string(), hex::encode(self.authorized_signer_key.to_bytes()));
            metadata.insert("signature".to_string(), hex::encode(signature.to_bytes()));
            metadata.insert(
                "signed_at".to_string(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .to_string(),
            );

            block.metadata = Some(metadata);
            Ok(())
        } else {
            Err(ConsensusError::BlockSigningFailed("No signing key available".to_string()))
        }
    }
}

impl crate::consensus::ConsensusEngine for PoAConsensus {
    fn validate_block(
        &self,
        block: &Block,
        _chain_state: &ChainState,
    ) -> Result<(), ConsensusError> {
        self.validate_block(block)
    }

    fn generate_block(
        &self,
        pending_transactions: &[Transaction],
        prev_block: &Block,
        _chain_state: &ChainState,
    ) -> Result<Block, ConsensusError> {
        info!("[CONSENSUS] Starting block generation");
        info!(
            "[CONSENSUS] Previous block: index={}, hash={}",
            prev_block.index,
            hex::encode(prev_block.hash)
        );
        info!("[CONSENSUS] Pending transactions: {}", pending_transactions.len());

        let index = prev_block.index + 1;
        let timestamp =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let prev_hash = prev_block.hash;

        info!("[CONSENSUS] New block parameters: index={}, timestamp={}", index, timestamp);

        // Select transactions with gas and size limits
        let mut transactions = Vec::new();
        let mut total_gas = 0u64;
        let mut total_size = 0usize;

        for tx in pending_transactions {
            total_gas += tx.gas_limit;
            total_size += std::mem::size_of_val(tx);
            total_size += tx.payload_size_estimate();

            if total_gas > self.block_gas_limit || total_size > self.block_size_limit {
                break;
            }
            transactions.push(tx.clone());
        }

        info!(
            "[CONSENSUS] Selected {} transactions (gas={}, size={} bytes)",
            transactions.len(),
            total_gas.min(self.block_gas_limit),
            total_size.min(self.block_size_limit)
        );

        let mut block = Block {
            index,
            timestamp,
            prev_hash,
            hash: [0u8; 32],
            nonce: 0,
            transactions,
            metadata: None,
        };

        info!("[CONSENSUS] Created block structure, calculating hash");
        // Calculate block hash
        block.hash = block
            .calculate_hash()
            .map_err(|e| ConsensusError::ValidationFailed(format!("Hash error: {:?}", e)))?;
        info!("[CONSENSUS] Block hash calculated: {}", hex::encode(block.hash));

        // Sign the block — mandatory for PoA
        if let Some(ref _signing_key) = self.signing_key {
            info!("[CONSENSUS] Signing block with authorized key");
            self.sign_block(&mut block)?;
            info!("[CONSENSUS] Block signed successfully");
        } else {
            return Err(ConsensusError::BlockSigningFailed(
                "No signing key available for block production".to_string(),
            ));
        }

        info!(
            "[CONSENSUS] Block generation completed: index={}, hash={}",
            block.index,
            hex::encode(block.hash)
        );
        Ok(block)
    }

    fn block_time_interval_ms(&self) -> u64 {
        self.block_time_interval_ms
    }
}
