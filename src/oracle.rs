/// Phase K — Resurgence Protocol oracle integration.
///
/// BaaLS acts as the immutable attestation registry between ChronoNode (dormancy detection)
/// and the Resurgence EVM contracts (reward minting).
///
/// Flow:
///   ChronoNode POST /api/v1/oracle/attest (DormancyProof)
///     → BaaLS verifies ChronoNode ed25519 signature
///     → BaaLS signs attestation with its own node key
///     → stores in contract-storage namespace (oracle_namespace_id)
///     → returns OracleAttestation (signed) to ChronoNode
///   ChronoNode submits OracleAttestation to Resurgence RewardDistributor on-chain
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Fixed 32-byte namespace used as the "contract ID" for oracle attestation storage.
/// Derived from SHA256("baals:oracle:v1") — no real WASM contract required; the REST
/// endpoint writes directly into this namespace via contract_storage_write.
pub fn oracle_namespace_id() -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"baals:oracle:v1");
    let result = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&result);
    arr
}

/// Dormancy proof as submitted by ChronoNode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DormancyProof {
    /// Protocol version tag, e.g. "chrononode:dormancy:v1"
    pub version: String,
    /// Chain identifier, e.g. "bitcoin", "dogecoin"
    pub chain_id: String,
    /// Address as a hex string (bytes may be UTF-8 for UTXO chains)
    pub address: String,
    pub dormant_since_block: u64,
    pub current_block: u64,
    pub threshold_blocks: u64,
    /// ChronoNode operator ed25519 verifying key (hex, 32 bytes)
    pub signer_pubkey: Option<String>,
    /// ChronoNode operator ed25519 signature over canonical_message() (hex, 64 bytes)
    pub signature: Option<String>,
    /// EVM address (checksummed hex, 0x-prefixed) that should receive RESURGE.
    /// Required for EVMSubmitter to call submitDormancyProof on the hub chain.
    pub evm_wallet: Option<String>,
    /// Legacy claim type index (matches LegacyClaimType enum in EVM)
    #[serde(default)]
    pub claim_type: u8,
    /// Confidence tier (1-7, lower is stronger)
    #[serde(default)]
    pub confidence_tier: u8,
    /// Source transaction hash for transfer/burn claims
    pub source_tx_hash: Option<String>,
    /// SP1 zkVM Groth16 proof bytes (hex-encoded)
    #[serde(default)]
    pub zk_proof: Option<String>,
    /// SP1 public inputs (hex-encoded)
    #[serde(default)]
    pub public_inputs: Option<String>,
}

/// BaaLS-signed oracle attestation returned to ChronoNode after validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleAttestation {
    pub proof: DormancyProof,
    /// BaaLS node ed25519 verifying key (hex, 32 bytes)
    pub baals_pubkey: String,
    /// BaaLS signature over canonical_message() (hex, 64 bytes)
    pub baals_signature: String,
    /// BaaLS block index at time of attestation (chain anchor)
    pub attested_at_block: u64,
    /// BaaLS block hash at time of attestation (hex, 32 bytes)
    #[serde(default)]
    pub baals_block_hash: String,
    /// Unique attestation ID (hash of proof + block)
    #[serde(default)]
    pub attestation_id: String,
    /// Canonical claim ID (computed from proof fields)
    pub claim_id: Option<String>,
    /// EVM tx hash after submission (set by EVMSubmitter)
    pub evm_tx_hash: Option<String>,
    /// BaaLS internal tx hash for the attestation record
    pub baals_tx_hash: Option<String>,
    /// Hash of the dormancy proof (for replay protection)
    pub proof_hash: Option<String>,
    /// Lifecycle status: pending, submitted, confirmed, failed
    pub status: String,
    /// Unix timestamp when attestation was created
    pub created_at: u64,
    /// Unix timestamp when attestation was submitted to EVM
    pub submitted_at: Option<u64>,
    /// Error message if submission failed
    pub error: Option<String>,
}

impl DormancyProof {
    /// Canonical byte string signed by ChronoNode and counter-signed by BaaLS.
    /// Format: {version}:{chain_id}:{address}:{dormant_since_be8}{current_be8}{threshold_be8}
    pub fn canonical_message(&self) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.extend_from_slice(self.version.as_bytes());
        msg.push(b':');
        msg.extend_from_slice(self.chain_id.as_bytes());
        msg.push(b':');
        msg.extend_from_slice(self.address.as_bytes());
        msg.push(b':');
        msg.extend_from_slice(&self.dormant_since_block.to_be_bytes());
        msg.extend_from_slice(&self.current_block.to_be_bytes());
        msg.extend_from_slice(&self.threshold_blocks.to_be_bytes());
        msg
    }

    /// Verify the ChronoNode operator signature on this proof.
    pub fn verify_chrononode_signature(&self) -> Result<(), Box<dyn std::error::Error>> {
        let pubkey_hex = self.signer_pubkey.as_deref().ok_or("missing signer_pubkey")?;
        let sig_hex = self.signature.as_deref().ok_or("missing signature")?;

        let pubkey_bytes = hex::decode(pubkey_hex).map_err(|e| format!("signer_pubkey: {}", e))?;
        let sig_bytes = hex::decode(sig_hex).map_err(|e| format!("signature: {}", e))?;

        if pubkey_bytes.len() != 32 {
            return Err("signer_pubkey must be 32 bytes".into());
        }
        if sig_bytes.len() != 64 {
            return Err("signature must be 64 bytes".into());
        }

        let mut pk_arr = [0u8; 32];
        pk_arr.copy_from_slice(&pubkey_bytes);
        let vk = VerifyingKey::from_bytes(&pk_arr).map_err(|e| format!("bad pubkey: {}", e))?;

        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let sig = Signature::from_bytes(&sig_arr);

        vk.verify(&self.canonical_message(), &sig)
            .map_err(|e| format!("signature verification failed: {}", e).into())
    }
}

/// Sign a validated DormancyProof with the BaaLS node key and return an OracleAttestation.
pub fn sign_attestation(
    proof: DormancyProof,
    signing_key: &SigningKey,
    attested_at_block: u64,
    baals_block_hash: [u8; 32],
) -> OracleAttestation {
    let vk = signing_key.verifying_key();
    let sig = signing_key.sign(&proof.canonical_message());

    let attestation_id = {
        let mut h = Sha256::new();
        h.update(b"attestation:");
        h.update(proof.chain_id.as_bytes());
        h.update(proof.address.as_bytes());
        h.update(&attested_at_block.to_be_bytes());
        h.update(&baals_block_hash);
        format!("0x{}", hex::encode(h.finalize()))
    };

    let claim_id = if proof.source_tx_hash.is_some() || proof.claim_type > 0 {
        let mut h = Sha256::new();
        h.update(b"claim:");
        h.update(proof.chain_id.as_bytes());
        h.update(proof.address.as_bytes());
        if let Some(evm) = &proof.evm_wallet {
            h.update(evm.as_bytes());
        }
        h.update(&proof.claim_type.to_be_bytes());
        if let Some(tx) = &proof.source_tx_hash {
            h.update(tx.as_bytes());
        }
        Some(format!("0x{}", hex::encode(h.finalize())))
    } else {
        None
    };

    let proof_hash = {
        let mut h = Sha256::new();
        h.update(b"proof:");
        h.update(proof.chain_id.as_bytes());
        h.update(proof.address.as_bytes());
        h.update(&proof.dormant_since_block.to_be_bytes());
        h.update(&proof.current_block.to_be_bytes());
        format!("0x{}", hex::encode(h.finalize()))
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    OracleAttestation {
        baals_pubkey: hex::encode(vk.to_bytes()),
        baals_signature: hex::encode(sig.to_bytes()),
        attested_at_block,
        baals_block_hash: hex::encode(baals_block_hash),
        proof,
        attestation_id,
        claim_id,
        evm_tx_hash: None,
        baals_tx_hash: None,
        proof_hash: Some(proof_hash),
        status: "pending".to_string(),
        created_at: now,
        submitted_at: None,
        error: None,
    }
}

/// Storage key for an oracle attestation within the oracle namespace.
/// Format: "oracle:attest:{chain_id}:{address}"
pub fn attestation_storage_key(chain_id: &str, address: &str) -> Vec<u8> {
    format!("oracle:attest:{}:{}", chain_id, address).into_bytes()
}

/// Storage key listing all attested addresses for a chain.
/// Format: "oracle:index:{chain_id}"
pub fn chain_index_key(chain_id: &str) -> Vec<u8> {
    format!("oracle:index:{}", chain_id).into_bytes()
}

/// Storage key listing all chain IDs that have at least one attestation.
/// Format: "oracle:chains"
pub fn chains_storage_key() -> Vec<u8> {
    b"oracle:chains".to_vec()
}
