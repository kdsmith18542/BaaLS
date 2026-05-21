use ed25519_dalek::{Signer, SigningKey};
use hex;
use log::{debug, info, warn};
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

    fn supports_signer_rotation(&self) -> bool {
        false
    }

    fn block_time_interval_ms(&self) -> u64 {
        5000
    }

    /// Add an authorized signer. Returns true if newly added.
    fn add_signer(&self, _pk: PublicKey) -> bool {
        false
    }

    /// Remove an authorized signer. Returns true if removed.
    fn remove_signer(&self, _pk: PublicKey) -> bool {
        false
    }

    /// List all authorized signers (excluding primary).
    fn list_signers(&self) -> Vec<PublicKey> {
        vec![]
    }

    /// Return the primary signer public key.
    fn primary_signer(&self) -> Option<PublicKey> {
        None
    }
}

pub struct PoAConsensus {
    authorized_signer_key: PublicKey,
    authorized_signers: std::sync::RwLock<Vec<PublicKey>>,
    block_time_interval_ms: u64,
    signing_key: Option<SigningKey>,
    pub block_gas_limit: u64,
    pub block_size_limit: usize,
    pub quorum_threshold: usize,
    pub round_robin: bool,
}

impl PoAConsensus {
    const MAX_FUTURE_BLOCK_TIMESTAMP_SECONDS: u64 = 10;
    pub fn new(authorized_signer_key: PublicKey, block_time_interval_ms: u64) -> Self {
        Self {
            authorized_signer_key,
            authorized_signers: std::sync::RwLock::new(Vec::new()),
            block_time_interval_ms,
            signing_key: None,
            block_gas_limit: 30_000_000,        // 30M gas per block
            block_size_limit: 10 * 1024 * 1024, // 10MB per block
            quorum_threshold: 1,                // default: single-signer
            round_robin: false,
        }
    }

    pub fn with_quorum_threshold(mut self, threshold: usize) -> Self {
        self.quorum_threshold = threshold.max(1);
        self
    }

    pub fn with_round_robin(mut self, enabled: bool) -> Self {
        self.round_robin = enabled;
        self
    }

    /// Returns the expected signer public key for the given block index
    /// under round-robin scheduling. All signers (primary + authorized) are
    /// included in the rotation.
    pub fn expected_signer_for_index(&self, block_index: u64) -> PublicKey {
        let signers = self.authorized_signers.read().unwrap();
        let all: Vec<&PublicKey> =
            std::iter::once(&self.authorized_signer_key).chain(signers.iter()).collect();
        *all[(block_index as usize) % all.len()]
    }

    pub fn with_signing_key(mut self, signing_key: SigningKey) -> Self {
        self.signing_key = Some(signing_key);
        self
    }

    pub fn add_authorized_signer(&self, pk: PublicKey) -> bool {
        let mut signers = self.authorized_signers.write().unwrap();
        if !signers.contains(&pk) {
            signers.push(pk);
            true
        } else {
            false
        }
    }

    pub fn remove_authorized_signer(&self, pk: PublicKey) -> bool {
        let mut signers = self.authorized_signers.write().unwrap();
        let before = signers.len();
        signers.retain(|k| *k != pk);
        signers.len() < before
    }

    pub fn authorized_signers_list(&self) -> Vec<PublicKey> {
        self.authorized_signers.read().unwrap().clone()
    }

    pub fn load_authorized_signers_from_storage(
        &self,
        storage: &dyn crate::storage::Storage,
    ) -> Result<(), crate::storage::StorageError> {
        let loaded = storage.get_authorized_signers()?;
        *self.authorized_signers.write().unwrap() = loaded;
        Ok(())
    }

    pub fn persist_authorized_signers_to_storage(
        &self,
        storage: &dyn crate::storage::Storage,
    ) -> Result<(), crate::storage::StorageError> {
        let signers = self.authorized_signers.read().unwrap();
        storage.put_authorized_signers(&signers)
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

        // Verify signer matches authorized key or authorized signers set
        let signer_hex = metadata.get("signer").ok_or_else(|| {
            ConsensusError::ValidationFailed("Signer not found in block metadata".to_string())
        })?;

        let primary_key_hex = hex::encode(self.authorized_signer_key.to_bytes());
        let is_primary = *signer_hex == primary_key_hex;

        // Snapshot signers under a short-lived read lock to avoid holding it
        // across nested calls (expected_signer_for_index, quorum check).
        let signers_snapshot: Vec<PublicKey> = self.authorized_signers.read().unwrap().clone();

        let verifier_pk = if is_primary {
            self.authorized_signer_key
        } else {
            match signers_snapshot.iter().find(|pk| hex::encode(pk.to_bytes()) == *signer_hex) {
                Some(pk) => *pk,
                None => {
                    warn!(
                        "[CONSENSUS] UnauthorizedSigner: block signer={} primary={} additional_count={} additional={:?}",
                        signer_hex,
                        primary_key_hex,
                        signers_snapshot.len(),
                        signers_snapshot.iter().map(|pk| hex::encode(pk.to_bytes())).collect::<Vec<_>>()
                    );
                    return Err(ConsensusError::UnauthorizedSigner);
                }
            }
        };

        // Round-robin check — with a 1-block grace window for liveness
        if self.round_robin && !signers_snapshot.is_empty() {
            let all_rr: Vec<&PublicKey> = std::iter::once(&self.authorized_signer_key)
                .chain(signers_snapshot.iter())
                .collect();
            let expected = *all_rr[(block.index as usize) % all_rr.len()];
            let expected_hex = hex::encode(expected.to_bytes());
            let prev_idx = block.index.saturating_sub(1) as usize % all_rr.len();
            let prev_hex = hex::encode(all_rr[prev_idx].to_bytes());
            if *signer_hex != expected_hex && *signer_hex != prev_hex {
                return Err(ConsensusError::ValidationFailed(format!(
                    "Round-robin violation: block {} expected signer {}, got {}",
                    block.index,
                    &expected_hex[..8],
                    &signer_hex[..8.min(signer_hex.len())]
                )));
            }
            info!(
                "[CONSENSUS] Round-robin slot {}: signer {}",
                block.index,
                &signer_hex[..8.min(signer_hex.len())]
            );
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

        verifier_pk.verify(&block.hash, &signature).map_err(|_| {
            ConsensusError::InvalidSignature(CryptoError::SignatureVerificationFailed)
        })?;

        // Nonce check — for PoA, nonce should be 0
        if block.nonce != 0 {
            return Err(ConsensusError::InvalidNonce);
        }

        // Quorum check — if threshold > 1, validate additional signatures
        if self.quorum_threshold > 1 {
            let all_signers: Vec<&PublicKey> = std::iter::once(&self.authorized_signer_key)
                .chain(signers_snapshot.iter())
                .collect();

            let mut valid_count = 1usize; // primary signer already verified above

            for (extra_signer_hex, extra_sig_bytes) in &block.quorum_signatures {
                let signer_pk = match all_signers
                    .iter()
                    .find(|pk| hex::encode(pk.to_bytes()) == *extra_signer_hex)
                {
                    Some(pk) => *pk,
                    None => continue, // ignore unknown signers
                };
                // Skip the primary signer if listed again in quorum_signatures
                if hex::encode(signer_pk.to_bytes()) == *signer_hex {
                    continue;
                }
                if extra_sig_bytes.len() == 64 {
                    if let Ok(sig) = ed25519_dalek::Signature::from_slice(extra_sig_bytes) {
                        if signer_pk.verify(&block.hash, &sig).is_ok() {
                            valid_count += 1;
                        }
                    }
                }
            }

            if valid_count < self.quorum_threshold {
                return Err(ConsensusError::ValidationFailed(format!(
                    "Quorum not met: {} valid signatures, {} required",
                    valid_count, self.quorum_threshold
                )));
            }
        }

        Ok(())
    }

    pub fn sign_block(&self, block: &mut Block) -> Result<(), ConsensusError> {
        if let Some(signing_key) = &self.signing_key {
            // Verify the private key corresponds to the authorized signer
            if signing_key.verifying_key().to_bytes() != self.authorized_signer_key.to_bytes() {
                return Err(ConsensusError::UnauthorizedSigner);
            }

            // Ensure signer is in metadata and hash is set
            // (For backward compat: if signer not yet in metadata, add it and recalculate hash)
            let mut metadata = block.metadata.clone().unwrap_or_default();
            let signer_hex = hex::encode(self.authorized_signer_key.to_bytes());
            if !metadata.contains_key("signer") {
                metadata.insert("signer".to_string(), signer_hex);
                block.metadata = Some(metadata.clone());
                // Recalculate hash with signer now in metadata
                block.hash = block
                    .calculate_hash()
                    .map_err(|e| ConsensusError::BlockSigningFailed(e.to_string()))?;
            }

            // Sign the block hash (which now includes signer identity from metadata)
            let signature = signing_key.sign(&block.hash);

            // Add signature and timestamp to block metadata
            let mut metadata = block.metadata.clone().unwrap_or_default();
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
        chain_state: &ChainState,
    ) -> Result<Block, ConsensusError> {
        info!("[CONSENSUS] Starting block generation");
        info!(
            "[CONSENSUS] Previous block: index={}, hash={}",
            prev_block.index,
            hex::encode(prev_block.hash)
        );
        debug!("[CONSENSUS] Pending transactions: {}", pending_transactions.len());

        let index = prev_block.index + 1;
        let timestamp =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let prev_hash = prev_block.hash;

        debug!("[CONSENSUS] New block parameters: index={}, timestamp={}", index, timestamp);

        // Select transactions with gas and size limits
        let mut transactions = Vec::new();
        let mut total_gas = 0u64;
        let mut total_size = 0usize;

        for tx in pending_transactions {
            total_gas = total_gas.saturating_add(tx.gas_limit);
            total_size = total_size
                .saturating_add(tx.payload_size_estimate() + std::mem::size_of::<Transaction>());

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

        // Determine signer: round-robin from all authorized keys, or primary
        let signers_snapshot: Vec<PublicKey> = self.authorized_signers.read().unwrap().clone();
        let block_signer_pk = if self.round_robin && !signers_snapshot.is_empty() {
            let all_rr: Vec<&PublicKey> = std::iter::once(&self.authorized_signer_key)
                .chain(signers_snapshot.iter())
                .collect();
            let chosen = *all_rr[(index as usize) % all_rr.len()];
            info!(
                "[CONSENSUS] Round-robin: block {} assigned to signer {}",
                index,
                &hex::encode(chosen.to_bytes())[..8]
            );
            chosen
        } else {
            self.authorized_signer_key
        };

        // Set signer in metadata BEFORE calculating hash
        // This ensures the block hash covers signer identity
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert("signer".to_string(), hex::encode(block_signer_pk.to_bytes()));

        let mut block = Block {
            index,
            timestamp,
            prev_hash,
            state_root: chain_state.accounts_root_hash,
            hash: [0u8; 32],
            nonce: 0,
            transactions,
            metadata: Some(metadata),
            total_gas_used: 0,
            signer: Some(hex::encode(block_signer_pk.to_bytes())),
            signature: None,
            quorum_signatures: Vec::new(),
        };

        debug!("[CONSENSUS] Created block structure, calculating hash");
        // Calculate block hash (now includes signer)
        block.hash = block
            .calculate_hash()
            .map_err(|e| ConsensusError::ValidationFailed(format!("Hash error: {:?}", e)))?;
        debug!("[CONSENSUS] Block hash calculated: {}", hex::encode(block.hash));

        // Sign the block hash — mandatory for PoA
        if let Some(ref _signing_key) = self.signing_key {
            debug!("[CONSENSUS] Signing block with authorized key");
            self.sign_block(&mut block)?;
            debug!("[CONSENSUS] Block signed successfully");
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

    fn add_signer(&self, pk: PublicKey) -> bool {
        self.add_authorized_signer(pk)
    }

    fn remove_signer(&self, pk: PublicKey) -> bool {
        self.remove_authorized_signer(pk)
    }

    fn list_signers(&self) -> Vec<PublicKey> {
        self.authorized_signers_list()
    }

    fn primary_signer(&self) -> Option<PublicKey> {
        Some(self.authorized_signer_key)
    }
}
