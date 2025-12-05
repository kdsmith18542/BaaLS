use thiserror::Error;
use ed25519_dalek::{Signer, SigningKey};

use crate::types::{Block, ChainState, Transaction, CryptoError, PublicKey};

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
}

pub trait ConsensusEngine: Send + Sync {
    fn validate_block(&self, block: &Block, chain_state: &ChainState) -> Result<(), ConsensusError>;
    fn generate_block(
        &self,
        pending_transactions: &[Transaction],
        prev_block: &Block,
        chain_state: &ChainState,
    ) -> Result<Block, ConsensusError>;
}

pub struct PoAConsensus {
    authorized_signer_key: PublicKey,
    _block_time_interval_ms: u64,
}

impl PoAConsensus {
    pub fn new(authorized_signer_key: PublicKey, block_time_interval_ms: u64) -> Self {
        Self {
            authorized_signer_key,
            _block_time_interval_ms: block_time_interval_ms,
        }
    }

    pub fn validate_block(&self, _block: &Block) -> Result<(), ConsensusError> {
        // For PoA, we just check if the block is signed by an authorized signer
        // In a real implementation, you'd check the signature against the authorized key
        
        // For now, just return Ok() - implement actual signature verification later
        Ok(())
    }

    pub fn sign_block(&self, block: &mut Block, private_key: &SigningKey) -> Result<(), ConsensusError> {
        // Verify the private key corresponds to the authorized signer
        if private_key.verifying_key().to_bytes() != self.authorized_signer_key.to_bytes() {
            return Err(ConsensusError::UnauthorizedSigner);
        }

        // Sign the block
        let _signature = private_key.sign(&block.hash);
        // TODO: Add signature to block metadata or create a signed block type
        
        Ok(())
    }
} 

impl crate::consensus::ConsensusEngine for PoAConsensus {
    fn validate_block(&self, block: &Block, _chain_state: &ChainState) -> Result<(), ConsensusError> {
        self.validate_block(block)
    }

    fn generate_block(
        &self,
        pending_transactions: &[Transaction],
        prev_block: &Block,
        _chain_state: &ChainState,
    ) -> Result<Block, ConsensusError> {
        if pending_transactions.is_empty() {
            return Err(ConsensusError::NoPendingTransactions);
        }
        let index = prev_block.index + 1;
        let timestamp = prev_block.timestamp + 1; // For MVP, just increment
        let prev_hash = prev_block.hash;
        let transactions = pending_transactions.to_vec();
        let mut block = Block {
            index,
            timestamp,
            prev_hash,
            hash: [0u8; 32],
            nonce: 0,
            transactions,
            metadata: None,
        };
        block.hash = block.calculate_hash().map_err(|e| ConsensusError::ValidationFailed(format!("Hash error: {:?}", e)))?;
        Ok(block)
    }
} 