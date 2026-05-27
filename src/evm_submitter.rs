/// Phase K.5 — EVM Submitter for Resurgence Protocol oracle integration.
///
/// Reads finalized BaaLS oracle attestations and submits them to the
/// Resurgence RewardDistributor on Arbitrum Sepolia via eth_sendRawTransaction.
///
/// Each attestation must have `evm_wallet` set (the EVM address to receive RESURGE).
/// Attestations without `evm_wallet` are skipped with a warning.
///
/// Transaction signing uses the secp256k1 key from the `evm_private_key_env` env var.
/// The corresponding address must hold DORMANCY_ORACLE_ROLE on RewardDistributor.
use crate::config::{OracleConfig, RelayConfig};
use crate::oracle::OracleAttestation;
use crate::storage::Storage;
use crate::ContractId;
use k256::ecdsa::{RecoveryId, Signature as K256Signature, SigningKey as K256SigningKey};
use k256::ecdsa::signature::hazmat::PrehashSigner;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use sha2::Digest;
use tiny_keccak::{Hasher, Keccak};

// ── ABI / selector ────────────────────────────────────────────────────────────

/// Compute keccak256 of a byte slice.
fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

/// 4-byte selector for submitLegacyClaim
fn submit_legacy_claim_selector() -> [u8; 4] {
    let sig = b"submitLegacyClaim((bytes32,bytes32,address,bytes32,bytes32,uint8,uint8,uint256,uint256,uint256,uint256),bytes)";
    let h = keccak256(sig);
    [h[0], h[1], h[2], h[3]]
}

/// 4-byte selector for submitZkDormancyClaim
fn submit_zk_dormancy_selector() -> [u8; 4] {
    let sig = b"submitZkDormancyClaim(bytes,bytes,string,uint64,uint64,uint64,(bytes32,bytes32,address,bytes32,bytes32,uint8,uint8,uint256,uint256,uint256,uint256))";
    let h = keccak256(sig);
    [h[0], h[1], h[2], h[3]]
}

/// ABI-encode a call to LegacyClaimRegistry.submitLegacyClaim. Returns selector + encoded args.
fn encode_submit_legacy_claim(
    attestation: &OracleAttestation,
    evm_wallet_hex: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let proof = &attestation.proof;

    // 1. sourceChainId: left-aligned bytes32
    let chain_id_bytes = proof.chain_id.as_bytes();
    let mut chain_id_slot = [0u8; 32];
    let copy_len = chain_id_bytes.len().min(32);
    chain_id_slot[..copy_len].copy_from_slice(&chain_id_bytes[..copy_len]);

    // 2. sourceAddressHash: Sha256 of address
    let mut hasher = sha2::Sha256::new();
    hasher.update(proof.address.as_bytes());
    let source_address_hash = hasher.finalize();
    let mut address_hash_slot = [0u8; 32];
    address_hash_slot.copy_from_slice(&source_address_hash);

    // 3. evmWallet: address (20 bytes, left-padded with 12 zeros)
    let wallet_hex = evm_wallet_hex.trim_start_matches("0x");
    let wallet_bytes = hex::decode(wallet_hex)?;
    if wallet_bytes.len() != 20 {
        return Err(format!("evm_wallet must be 20 bytes, got {}", wallet_bytes.len()).into());
    }
    let mut wallet_slot = [0u8; 32];
    wallet_slot[12..].copy_from_slice(&wallet_bytes);

    // 4. proofHash: bytes32
    let mut proof_hash_slot = [0u8; 32];
    if let Some(ph_str) = &attestation.proof_hash {
        let ph_bytes = hex::decode(ph_str.trim_start_matches("0x"))?;
        let ph_copy = ph_bytes.len().min(32);
        proof_hash_slot[..ph_copy].copy_from_slice(&ph_bytes[..ph_copy]);
    }

    // 5. sourceTxHash: bytes32
    let mut source_tx_hash_slot = [0u8; 32];
    if let Some(tx_str) = &proof.source_tx_hash {
        let tx_bytes = hex::decode(tx_str.trim_start_matches("0x"))?;
        let tx_copy = tx_bytes.len().min(32);
        source_tx_hash_slot[..tx_copy].copy_from_slice(&tx_bytes[..tx_copy]);
    }

    // 6. claimType: uint8 -> uint256
    let mut claim_type_slot = [0u8; 32];
    claim_type_slot[31] = proof.claim_type;

    // 7. confidenceTier: uint8 -> uint256
    let mut confidence_tier_slot = [0u8; 32];
    confidence_tier_slot[31] = proof.confidence_tier;

    // 8. lastSeenTimestamp: uint256
    let mut timestamp_slot = [0u8; 32];
    timestamp_slot[24..].copy_from_slice(&attestation.created_at.to_be_bytes());

    // 9. dormancySeconds: uint256
    let blocks = proof.current_block.saturating_sub(proof.dormant_since_block);
    let block_time = if proof.chain_id.to_lowercase().contains("bitcoin") || proof.chain_id.to_lowercase().contains("btc") {
        600
    } else if proof.chain_id.to_lowercase().contains("doge") {
        60
    } else {
        60
    };
    let dormancy_seconds = blocks * block_time;
    let mut dormancy_slot = [0u8; 32];
    dormancy_slot[24..].copy_from_slice(&dormancy_seconds.to_be_bytes());

    // 10. rewardAmount: uint256
    let base_reward: u64 = 1000;
    let conf_mult = match proof.claim_type {
        2 => 1.10, // BurnProof
        3 => 0.90, // LockProof
        8 => 1.00, // ZkDormancyProof
        1 => 1.00, // TransferToVault
        4 => 0.80, // SignatureDormancyProof
        5 => 0.60, // PublicRpcEvidenceProof
        6 => 0.40, // ExplorerEvidenceProof
        _ => 1.00,
    };
    let dorm_mult = if dormancy_seconds >= 315_360_000 {
        3.0
    } else if dormancy_seconds >= 157_680_000 {
        2.0
    } else if dormancy_seconds >= 94_608_000 {
        1.5
    } else if dormancy_seconds >= 31_536_000 {
        1.0
    } else {
        0.5
    };
    let reward_tokens = (base_reward as f64 * conf_mult * dorm_mult) as u128;
    let reward_wei = reward_tokens * 1_000_000_000_000_000_000;
    let mut reward_slot = [0u8; 32];
    reward_slot[16..].copy_from_slice(&reward_wei.to_be_bytes());

    // 11. campaignId: uint256
    let campaign_id_slot = [0u8; 32];

    // 12. offset of baalsAttestation (bytes) = 384 (12 * 32)
    let mut offset_slot = [0u8; 32];
    offset_slot[30..32].copy_from_slice(&384u16.to_be_bytes());

    // 13. length of baalsAttestation (bytes)
    let att_sig_bytes = hex::decode(&attestation.baals_signature)?;
    let mut att_len_slot = [0u8; 32];
    att_len_slot[24..].copy_from_slice(&(att_sig_bytes.len() as u64).to_be_bytes());

    // 14. data of baalsAttestation (padded to 32-byte boundary)
    let padded_att_len = (att_sig_bytes.len() + 31) / 32 * 32;
    let mut att_padded = vec![0u8; padded_att_len];
    att_padded[..att_sig_bytes.len()].copy_from_slice(&att_sig_bytes);

    let mut calldata = Vec::new();
    calldata.extend_from_slice(&submit_legacy_claim_selector());
    calldata.extend_from_slice(&chain_id_slot);
    calldata.extend_from_slice(&address_hash_slot);
    calldata.extend_from_slice(&wallet_slot);
    calldata.extend_from_slice(&proof_hash_slot);
    calldata.extend_from_slice(&source_tx_hash_slot);
    calldata.extend_from_slice(&claim_type_slot);
    calldata.extend_from_slice(&confidence_tier_slot);
    calldata.extend_from_slice(&timestamp_slot);
    calldata.extend_from_slice(&dormancy_slot);
    calldata.extend_from_slice(&reward_slot);
    calldata.extend_from_slice(&campaign_id_slot);
    calldata.extend_from_slice(&offset_slot);
    calldata.extend_from_slice(&att_len_slot);
    calldata.extend_from_slice(&att_padded);

    Ok(calldata)
}

/// ABI-encode a call to submitZkDormancyClaim.
fn encode_submit_zk_dormancy(
    groth16_proof: &[u8],
    public_inputs: &[u8],
    wallet_address: &str,
    dormant_since_block: u64,
    current_block: u64,
    threshold_blocks: u64,
    attestation: &OracleAttestation,
    evm_wallet_hex: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let proof = &attestation.proof;

    // 1. sourceChainId: left-aligned bytes32
    let chain_id_bytes = proof.chain_id.as_bytes();
    let mut chain_id_slot = [0u8; 32];
    let copy_len = chain_id_bytes.len().min(32);
    chain_id_slot[..copy_len].copy_from_slice(&chain_id_bytes[..copy_len]);

    // 2. sourceAddressHash: Sha256 of address
    let mut hasher = sha2::Sha256::new();
    hasher.update(proof.address.as_bytes());
    let source_address_hash = hasher.finalize();
    let mut address_hash_slot = [0u8; 32];
    address_hash_slot.copy_from_slice(&source_address_hash);

    // 3. evmWallet: address (20 bytes, left-padded with 12 zeros)
    let wallet_hex = evm_wallet_hex.trim_start_matches("0x");
    let wallet_bytes = hex::decode(wallet_hex)?;
    if wallet_bytes.len() != 20 {
        return Err(format!("evm_wallet must be 20 bytes, got {}", wallet_bytes.len()).into());
    }
    let mut wallet_slot = [0u8; 32];
    wallet_slot[12..].copy_from_slice(&wallet_bytes);

    // 4. proofHash: bytes32
    let mut proof_hash_slot = [0u8; 32];
    if let Some(ph_str) = &attestation.proof_hash {
        let ph_bytes = hex::decode(ph_str.trim_start_matches("0x"))?;
        let ph_copy = ph_bytes.len().min(32);
        proof_hash_slot[..ph_copy].copy_from_slice(&ph_bytes[..ph_copy]);
    }

    // 5. sourceTxHash: bytes32
    let mut source_tx_hash_slot = [0u8; 32];
    if let Some(tx_str) = &proof.source_tx_hash {
        let tx_bytes = hex::decode(tx_str.trim_start_matches("0x"))?;
        let tx_copy = tx_bytes.len().min(32);
        source_tx_hash_slot[..tx_copy].copy_from_slice(&tx_bytes[..tx_copy]);
    }

    // 6. claimType: uint8 -> uint256
    let mut claim_type_slot = [0u8; 32];
    claim_type_slot[31] = proof.claim_type;

    // 7. confidenceTier: uint8 -> uint256
    let mut confidence_tier_slot = [0u8; 32];
    confidence_tier_slot[31] = proof.confidence_tier;

    // 8. lastSeenTimestamp: uint256
    let mut timestamp_slot = [0u8; 32];
    timestamp_slot[24..].copy_from_slice(&attestation.created_at.to_be_bytes());

    // 9. dormancySeconds: uint256
    let blocks = proof.current_block.saturating_sub(proof.dormant_since_block);
    let block_time = if proof.chain_id.to_lowercase().contains("bitcoin") || proof.chain_id.to_lowercase().contains("btc") {
        600
    } else if proof.chain_id.to_lowercase().contains("doge") {
        60
    } else {
        60
    };
    let dormancy_seconds = blocks * block_time;
    let mut dormancy_slot = [0u8; 32];
    dormancy_slot[24..].copy_from_slice(&dormancy_seconds.to_be_bytes());

    // 10. rewardAmount: uint256
    let base_reward: u64 = 1000;
    let conf_mult = match proof.claim_type {
        2 => 1.10, // BurnProof
        3 => 0.90, // LockProof
        8 => 1.00, // ZkDormancyProof
        1 => 1.00, // TransferToVault
        4 => 0.80, // SignatureDormancyProof
        5 => 0.60, // PublicRpcEvidenceProof
        6 => 0.40, // ExplorerEvidenceProof
        _ => 1.00,
    };
    let dorm_mult = if dormancy_seconds >= 315_360_000 {
        3.0
    } else if dormancy_seconds >= 157_680_000 {
        2.0
    } else if dormancy_seconds >= 94_608_000 {
        1.5
    } else if dormancy_seconds >= 31_536_000 {
        1.0
    } else {
        0.5
    };
    let reward_tokens = (base_reward as f64 * conf_mult * dorm_mult) as u128;
    let reward_wei = reward_tokens * 1_000_000_000_000_000_000;
    let mut reward_slot = [0u8; 32];
    reward_slot[16..].copy_from_slice(&reward_wei.to_be_bytes());

    // 11. campaignId: uint256
    let campaign_id_slot = [0u8; 32];

    // Static integers for function arguments (dormantSinceBlock, currentBlock, thresholdBlocks)
    let mut dormant_since_slot = [0u8; 32];
    dormant_since_slot[24..].copy_from_slice(&dormant_since_block.to_be_bytes());
    let mut current_block_slot = [0u8; 32];
    current_block_slot[24..].copy_from_slice(&current_block.to_be_bytes());
    let mut threshold_blocks_slot = [0u8; 32];
    threshold_blocks_slot[24..].copy_from_slice(&threshold_blocks.to_be_bytes());

    // Dynamic arguments offsets:
    let static_head_len: u64 = 17 * 32;

    let groth16_offset = static_head_len;
    let public_inputs_offset = groth16_offset + 32 + ((groth16_proof.len() as u64 + 31) / 32 * 32);
    let wallet_address_bytes = wallet_address.as_bytes();
    let wallet_address_offset = public_inputs_offset + 32 + ((public_inputs.len() as u64 + 31) / 32 * 32);

    let mut groth16_offset_slot = [0u8; 32];
    groth16_offset_slot[24..].copy_from_slice(&groth16_offset.to_be_bytes());
    let mut public_inputs_offset_slot = [0u8; 32];
    public_inputs_offset_slot[24..].copy_from_slice(&public_inputs_offset.to_be_bytes());
    let mut wallet_address_offset_slot = [0u8; 32];
    wallet_address_offset_slot[24..].copy_from_slice(&wallet_address_offset.to_be_bytes());

    // Tails
    let mut groth16_len_slot = [0u8; 32];
    groth16_len_slot[24..].copy_from_slice(&(groth16_proof.len() as u64).to_be_bytes());
    let groth16_padded_len = (groth16_proof.len() + 31) / 32 * 32;
    let mut groth16_padded = vec![0u8; groth16_padded_len];
    groth16_padded[..groth16_proof.len()].copy_from_slice(groth16_proof);

    let mut pi_len_slot = [0u8; 32];
    pi_len_slot[24..].copy_from_slice(&(public_inputs.len() as u64).to_be_bytes());
    let pi_padded_len = (public_inputs.len() + 31) / 32 * 32;
    let mut pi_padded = vec![0u8; pi_padded_len];
    pi_padded[..public_inputs.len()].copy_from_slice(public_inputs);

    let mut wallet_addr_len_slot = [0u8; 32];
    wallet_addr_len_slot[24..].copy_from_slice(&(wallet_address_bytes.len() as u64).to_be_bytes());
    let wallet_addr_padded_len = (wallet_address_bytes.len() + 31) / 32 * 32;
    let mut wallet_addr_padded = vec![0u8; wallet_addr_padded_len];
    wallet_addr_padded[..wallet_address_bytes.len()].copy_from_slice(wallet_address_bytes);

    let mut calldata = Vec::new();
    calldata.extend_from_slice(&submit_zk_dormancy_selector());
    
    calldata.extend_from_slice(&groth16_offset_slot);
    calldata.extend_from_slice(&public_inputs_offset_slot);
    calldata.extend_from_slice(&wallet_address_offset_slot);
    calldata.extend_from_slice(&dormant_since_slot);
    calldata.extend_from_slice(&current_block_slot);
    calldata.extend_from_slice(&threshold_blocks_slot);
    
    calldata.extend_from_slice(&chain_id_slot);
    calldata.extend_from_slice(&address_hash_slot);
    calldata.extend_from_slice(&wallet_slot);
    calldata.extend_from_slice(&proof_hash_slot);
    calldata.extend_from_slice(&source_tx_hash_slot);
    calldata.extend_from_slice(&claim_type_slot);
    calldata.extend_from_slice(&confidence_tier_slot);
    calldata.extend_from_slice(&timestamp_slot);
    calldata.extend_from_slice(&dormancy_slot);
    calldata.extend_from_slice(&reward_slot);
    calldata.extend_from_slice(&campaign_id_slot);

    calldata.extend_from_slice(&groth16_len_slot);
    calldata.extend_from_slice(&groth16_padded);
    calldata.extend_from_slice(&pi_len_slot);
    calldata.extend_from_slice(&pi_padded);
    calldata.extend_from_slice(&wallet_addr_len_slot);
    calldata.extend_from_slice(&wallet_addr_padded);

    Ok(calldata)
}

// ── RLP encoding ──────────────────────────────────────────────────────────────

fn rlp_encode_bytes(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return vec![0x80];
    }
    if data.len() == 1 && data[0] < 0x80 {
        return data.to_vec();
    }
    let mut out = Vec::new();
    if data.len() <= 55 {
        out.push(0x80 + data.len() as u8);
    } else {
        let len_bytes = encode_length_be(data.len());
        out.push(0xb7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
    }
    out.extend_from_slice(data);
    out
}

fn rlp_encode_uint(value: u64) -> Vec<u8> {
    if value == 0 {
        return vec![0x80];
    }
    let be = value.to_be_bytes();
    let bytes = strip_leading_zeros(&be);
    rlp_encode_bytes(bytes)
}

fn rlp_encode_bigint_bytes(data: &[u8]) -> Vec<u8> {
    let stripped = strip_leading_zeros(data);
    rlp_encode_bytes(stripped)
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.iter().flat_map(|v| v.iter().copied()).collect();
    let mut out = Vec::new();
    if payload.len() <= 55 {
        out.push(0xc0 + payload.len() as u8);
    } else {
        let len_bytes = encode_length_be(payload.len());
        out.push(0xf7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
    }
    out.extend_from_slice(&payload);
    out
}

fn encode_length_be(n: usize) -> Vec<u8> {
    let bytes = (n as u64).to_be_bytes();
    let stripped = strip_leading_zeros(&bytes);
    stripped.to_vec()
}

fn strip_leading_zeros(data: &[u8]) -> &[u8] {
    let first_nonzero = data.iter().position(|&b| b != 0).unwrap_or(data.len());
    &data[first_nonzero..]
}

// ── Transaction signing ───────────────────────────────────────────────────────

/// Build and sign an EIP-155 legacy transaction.
/// Returns the hex-encoded raw transaction (0x-prefixed).
fn sign_legacy_tx(
    sk: &K256SigningKey,
    nonce: u64,
    gas_price_wei: u64,
    gas_limit: u64,
    to: &str,            // 20-byte hex address, 0x-prefixed
    calldata: &[u8],
    chain_id: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let to_bytes = hex::decode(to.trim_start_matches("0x"))?;
    if to_bytes.len() != 20 {
        return Err("to address must be 20 bytes".into());
    }

    // Build the pre-image for EIP-155 signing:
    // RLP([nonce, gasPrice, gasLimit, to, value=0, data, chainId, 0, 0])
    let pre_image = rlp_encode_list(&[
        rlp_encode_uint(nonce),
        rlp_encode_uint(gas_price_wei),
        rlp_encode_uint(gas_limit),
        rlp_encode_bytes(&to_bytes),
        rlp_encode_uint(0),   // value = 0
        rlp_encode_bytes(calldata),
        rlp_encode_uint(chain_id),
        rlp_encode_uint(0),
        rlp_encode_uint(0),
    ]);

    // Hash the RLP pre-image with keccak256, then sign the 32-byte hash
    let tx_hash = keccak256(&pre_image);
    let (sig, recid): (K256Signature, RecoveryId) = sk
        .sign_prehash(&tx_hash)
        .map_err(|e| format!("signing failed: {}", e))?;

    let r_bytes: [u8; 32] = sig.r().to_bytes().into();
    let s_bytes: [u8; 32] = sig.s().to_bytes().into();
    let recovery = recid.to_byte() as u64;
    let v = chain_id * 2 + 35 + recovery;

    // Encode signed transaction:
    // RLP([nonce, gasPrice, gasLimit, to, value, data, v, r, s])
    let signed = rlp_encode_list(&[
        rlp_encode_uint(nonce),
        rlp_encode_uint(gas_price_wei),
        rlp_encode_uint(gas_limit),
        rlp_encode_bytes(&to_bytes),
        rlp_encode_uint(0),
        rlp_encode_bytes(calldata),
        rlp_encode_uint(v),
        rlp_encode_bigint_bytes(&r_bytes),
        rlp_encode_bigint_bytes(&s_bytes),
    ]);

    Ok(format!("0x{}", hex::encode(&signed)))
}

// ── JSON-RPC helpers ──────────────────────────────────────────────────────────

fn rpc_call(
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    let resp: serde_json::Value = ureq::post(rpc_url)
        .set("Content-Type", "application/json")
        .send_json(&body)?
        .into_json()?;
    if let Some(err) = resp.get("error") {
        return Err(format!("RPC error: {}", err).into());
    }
    Ok(resp["result"].clone())
}

fn get_nonce(rpc_url: &str, address: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let result = rpc_call(
        rpc_url,
        "eth_getTransactionCount",
        serde_json::json!([address, "latest"]),
    )?;
    let hex_str = result.as_str().ok_or("nonce not a string")?;
    let n = u64::from_str_radix(hex_str.trim_start_matches("0x"), 16)?;
    Ok(n)
}

fn get_gas_price(rpc_url: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let result = rpc_call(rpc_url, "eth_gasPrice", serde_json::json!([]))?;
    let hex_str = result.as_str().ok_or("gasPrice not a string")?;
    let gp = u64::from_str_radix(hex_str.trim_start_matches("0x"), 16)?;
    // Add 20% buffer to avoid stuck transactions
    Ok(gp * 12 / 10)
}

fn send_raw_tx(rpc_url: &str, raw_tx_hex: &str) -> Result<String, Box<dyn std::error::Error>> {
    let result = rpc_call(
        rpc_url,
        "eth_sendRawTransaction",
        serde_json::json!([raw_tx_hex]),
    )?;
    let tx_hash = result.as_str().ok_or("tx hash not a string")?.to_string();
    Ok(tx_hash)
}

/// Poll for tx receipt and return true if status=1 (success), false if reverted, None if not yet mined.
fn check_tx_success(rpc_url: &str, tx_hash: &str) -> Option<bool> {
    let result = rpc_call(
        rpc_url,
        "eth_getTransactionReceipt",
        serde_json::json!([tx_hash]),
    ).ok()?;
    if result.is_null() {
        return None; // not yet mined
    }
    let status = result["status"].as_str()?;
    Some(status == "0x1")
}

/// Derive the EVM address (checksummed hex) from a secp256k1 signing key.
fn evm_address(sk: &K256SigningKey) -> String {
    let vk = sk.verifying_key();
    // Uncompressed public key: 0x04 ++ x ++ y (65 bytes); hash without the 04 prefix
    let point = vk.to_encoded_point(false);
    let pubkey_bytes = &point.as_bytes()[1..]; // strip 0x04 prefix
    let hash = keccak256(pubkey_bytes);
    format!("0x{}", hex::encode(&hash[12..]))
}

// ── Submission state ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionState {
    pub chain_id: String,
    pub address: String,
    pub submitted_at_block: u64,
    pub evm_tx_hash: Option<String>,
    /// "pending" | "confirmed" | "failed"
    pub status: String,
    pub attempts: u32,
    pub last_attempt_time: u64,
}

impl SubmissionState {
    fn is_eligible_for_retry(&self) -> bool {
        if self.status == "confirmed" {
            return false;
        }
        if self.attempts == 0 {
            return true;
        }
        if self.attempts >= 5 {
            return false;
        }
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(self.last_attempt_time);
        let backoff_secs = (2_u64).saturating_pow(self.attempts) * 60;
        elapsed >= backoff_secs
    }
}

// ── EVMSubmitter ──────────────────────────────────────────────────────────────

pub struct EVMSubmitter {
    config: OracleConfig,
    signing_key: K256SigningKey,
    evm_address: String,
    submission_states: Arc<tokio::sync::Mutex<HashMap<String, SubmissionState>>>,
}

impl EVMSubmitter {
    /// Create an EVMSubmitter, reading the secp256k1 key from the env var named in config.
    /// Returns `None` if oracle is disabled or the key env var is unset/invalid.
    pub fn new(config: OracleConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        let key_hex = std::env::var(&config.evm_private_key_env).ok()?;
        let key_bytes = hex::decode(key_hex.trim()).ok()?;
        let sk = K256SigningKey::from_slice(&key_bytes).ok()?;
        let addr = evm_address(&sk);
        info!("[EVM] Submitter initialized. Signing address: {}", addr);
        Some(Self {
            config,
            signing_key: sk,
            evm_address: addr,
            submission_states: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        })
    }

    /// Background loop — polls every 30 s.
    pub async fn run(&self, storage: &dyn Storage) {
        if self.config.evm_rpc.is_empty() || self.config.legacy_claim_registry.is_empty() {
            error!("[EVM] Missing evm_rpc or legacy_claim_registry config — submitter disabled");
            return;
        }
        loop {
            if let Err(e) = self.poll_and_submit(storage).await {
                error!("[EVM] Poll/submit error: {}", e);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        }
    }

    async fn poll_and_submit(
        &self,
        storage: &dyn Storage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let oracle_ns = ContractId::from_bytes(&crate::oracle::oracle_namespace_id());

        let chains_raw =
            storage.contract_storage_read(&oracle_ns, &crate::oracle::chains_storage_key())?;
        let chain_ids: Vec<String> = chains_raw
            .map(|r| serde_json::from_slice(&r).unwrap_or_default())
            .unwrap_or_default();

        let mut states = self.submission_states.lock().await;

        for chain_id in chain_ids {
            let index_raw = storage
                .contract_storage_read(&oracle_ns, &crate::oracle::chain_index_key(&chain_id))?;
            let addresses: Vec<String> = index_raw
                .map(|r| serde_json::from_slice(&r).unwrap_or_default())
                .unwrap_or_default();

            for address in addresses {
                let state_key = format!("evm:submission:{}:{}", chain_id, address);

                let state = states.get(&state_key).cloned().unwrap_or(SubmissionState {
                    chain_id: chain_id.clone(),
                    address: address.clone(),
                    submitted_at_block: 0,
                    evm_tx_hash: None,
                    status: "pending".to_string(),
                    attempts: 0,
                    last_attempt_time: 0,
                });

                if state.status == "confirmed" {
                    continue;
                }
                if state.attempts > 0 && !state.is_eligible_for_retry() {
                    continue;
                }

                let attest_key =
                    crate::oracle::attestation_storage_key(&chain_id, &address);
                let attest_raw = storage.contract_storage_read(&oracle_ns, &attest_key)?;
                let attestation: OracleAttestation = match attest_raw {
                    Some(raw) => serde_json::from_slice(&raw)?,
                    None => continue,
                };

                let evm_wallet = match &attestation.proof.evm_wallet {
                    Some(w) => w.clone(),
                    None => {
                        warn!(
                            "[EVM] Skipping attestation chain={} addr={}: no evm_wallet set",
                            chain_id, address
                        );
                        continue;
                    }
                };

                let is_zk = attestation.proof.claim_type == 8
                    && attestation.proof.zk_proof.is_some()
                    && attestation.proof.public_inputs.is_some();

                let new_state = if is_zk {
                    match self.submit_sp1_attestation(
                        &attestation,
                        &evm_wallet,
                        &hex::decode(attestation.proof.zk_proof.as_deref().unwrap_or(""))?,
                        &hex::decode(attestation.proof.public_inputs.as_deref().unwrap_or(""))?,
                    ) {
                        Ok(tx_hash) => {
                            info!(
                                "[EVM] Submitted SP1: chain={} addr={} evm={} tx={}",
                                chain_id, address, evm_wallet, tx_hash
                            );
                            SubmissionState {
                                evm_tx_hash: Some(tx_hash),
                                status: "pending".to_string(),
                                attempts: state.attempts + 1,
                                last_attempt_time: unix_now(),
                                ..state
                            }
                        }
                        Err(e) => {
                            warn!(
                                "[EVM] SP1 submit failed (attempt {}): chain={} addr={} err={}",
                                state.attempts + 1, chain_id, address, e
                            );
                            SubmissionState {
                                status: if state.attempts >= 4 { "failed".to_string() } else { "pending".to_string() },
                                attempts: state.attempts + 1,
                                last_attempt_time: unix_now(),
                                ..state
                            }
                        }
                    }
                } else {
                    match self.submit_attestation(&attestation, &evm_wallet) {
                        Ok(tx_hash) => {
                            info!(
                                "[EVM] Submitted: chain={} addr={} evm={} tx={}",
                                chain_id, address, evm_wallet, tx_hash
                            );
                            SubmissionState {
                                evm_tx_hash: Some(tx_hash),
                                status: "pending".to_string(),
                                attempts: state.attempts + 1,
                                last_attempt_time: unix_now(),
                                ..state
                            }
                        }
                        Err(e) => {
                            warn!(
                                "[EVM] Submit failed (attempt {}): chain={} addr={} err={}",
                                state.attempts + 1,
                                chain_id,
                                address,
                                e
                            );
                            SubmissionState {
                                status: if state.attempts >= 4 {
                                    "failed".to_string()
                                } else {
                                    "pending".to_string()
                                },
                                attempts: state.attempts + 1,
                                last_attempt_time: unix_now(),
                                ..state
                            }
                        }
                    }
                };

                states.insert(state_key, new_state);
            }
        }

        Ok(())
    }

    /// Build, sign, and send a submitLegacyClaim transaction.
    fn submit_attestation(
        &self,
        attestation: &OracleAttestation,
        evm_wallet: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let calldata = encode_submit_legacy_claim(attestation, evm_wallet)?;

        let nonce = get_nonce(&self.config.evm_rpc, &self.evm_address)?;
        let gas_price = get_gas_price(&self.config.evm_rpc)?;

        debug!(
            "[EVM] Building tx: nonce={} gasPrice={} to={}",
            nonce, gas_price, self.config.legacy_claim_registry
        );

        let raw_tx = sign_legacy_tx(
            &self.signing_key,
            nonce,
            gas_price,
            500_000,
            &self.config.legacy_claim_registry,
            &calldata,
            self.config.evm_chain_id,
        )?;

        let tx_hash = send_raw_tx(&self.config.evm_rpc, &raw_tx)?;
        Ok(tx_hash)
    }

    /// Build, sign, and send a submitZkDormancyClaim transaction for SP1 zkVM proofs.
    fn submit_sp1_attestation(
        &self,
        attestation: &OracleAttestation,
        evm_wallet: &str,
        groth16_proof: &[u8],
        public_inputs: &[u8],
    ) -> Result<String, Box<dyn std::error::Error>> {
        let proof = &attestation.proof;

        let calldata = encode_submit_zk_dormancy(
            groth16_proof,
            public_inputs,
            &proof.address,
            proof.dormant_since_block,
            proof.current_block,
            proof.threshold_blocks,
            attestation,
            evm_wallet,
        )?;

        let nonce = get_nonce(&self.config.evm_rpc, &self.evm_address)?;
        let gas_price = get_gas_price(&self.config.evm_rpc)?;

        debug!(
            "[EVM] Building SP1 tx: nonce={} gasPrice={} to={}",
            nonce, gas_price, self.config.legacy_claim_registry
        );

        let raw_tx = sign_legacy_tx(
            &self.signing_key,
            nonce,
            gas_price,
            800_000,
            &self.config.legacy_claim_registry,
            &calldata,
            self.config.evm_chain_id,
        )?;

        let tx_hash = send_raw_tx(&self.config.evm_rpc, &raw_tx)?;
        Ok(tx_hash)
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Phase L: BaaLS Relay Bridge Watcher ──────────────────────────────────────
//
// Watches spoke chains for BridgeClaimRequested(address,uint256,uint256) events
// emitted by CrossChainSender.bridgeClaimRelay(). For each new event, calls
// RewardDistributor.mintForRelay(user, amount, crossChainSender, nonce) on the hub.
//
// Deduplication: (crossChainSender_lower, nonce_hex) pairs are stored in a JSON
// state file that also tracks the last-scanned block per spoke so restarts resume
// from where they left off rather than rescanning from genesis.

fn bridge_claim_requested_topic() -> [u8; 32] {
    keccak256(b"BridgeClaimRequested(address,uint256,uint256)")
}

fn mint_for_relay_selector() -> [u8; 4] {
    let h = keccak256(b"mintForRelay(address,uint256,address,uint256)");
    [h[0], h[1], h[2], h[3]]
}

/// ABI-encode mintForRelay(address user, uint256 amount, address crossChainSender, uint256 nonce).
/// All args are fixed-size, so calldata = 4 (selector) + 4×32 = 132 bytes.
fn encode_mint_for_relay(
    user: &str,
    amount_slot: &[u8; 32],
    cross_chain_sender: &str,
    nonce_slot: &[u8; 32],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let user_raw = hex::decode(user.trim_start_matches("0x"))?;
    if user_raw.len() != 20 {
        return Err(format!("user must be 20 bytes, got {}", user_raw.len()).into());
    }
    let sender_raw = hex::decode(cross_chain_sender.trim_start_matches("0x"))?;
    if sender_raw.len() != 20 {
        return Err(format!("crossChainSender must be 20 bytes, got {}", sender_raw.len()).into());
    }

    let mut user_slot = [0u8; 32];
    user_slot[12..].copy_from_slice(&user_raw);
    let mut sender_slot = [0u8; 32];
    sender_slot[12..].copy_from_slice(&sender_raw);

    let mut calldata = Vec::with_capacity(132);
    calldata.extend_from_slice(&mint_for_relay_selector());
    calldata.extend_from_slice(&user_slot);
    calldata.extend_from_slice(amount_slot);
    calldata.extend_from_slice(&sender_slot);
    calldata.extend_from_slice(nonce_slot);
    Ok(calldata)
}

/// On-disk state for the relay watcher (JSON).
#[derive(Debug, Serialize, Deserialize, Default)]
struct RelayPersist {
    /// Last block scanned per spoke label — resume point on restart.
    #[serde(default)]
    last_scanned_block: HashMap<String, u64>,
    /// Processed dedup keys: "<sender_lower>:<nonce_hex>".
    #[serde(default)]
    processed: HashSet<String>,
}

pub struct RelayWatcher {
    relay_cfg: RelayConfig,
    oracle_cfg: OracleConfig,
    signing_key: K256SigningKey,
    evm_address: String,
}

impl RelayWatcher {
    /// Create a RelayWatcher. Returns `None` if relay is disabled, misconfigured, or
    /// the signing key env var is absent/invalid.
    pub fn new(relay_cfg: RelayConfig, oracle_cfg: OracleConfig) -> Option<Self> {
        if !relay_cfg.enabled {
            return None;
        }
        if relay_cfg.spokes.is_empty() {
            warn!("[Relay] relay.enabled=true but no spokes configured — watcher disabled");
            return None;
        }
        if oracle_cfg.reward_distributor.is_empty() || oracle_cfg.evm_rpc.is_empty() {
            warn!("[Relay] relay.enabled=true but oracle.reward_distributor or oracle.evm_rpc is empty — watcher disabled");
            return None;
        }
        let key_hex = match std::env::var(&oracle_cfg.evm_private_key_env) {
            Ok(k) => k,
            Err(_) => {
                warn!("[Relay] Env var '{}' not set — watcher disabled", oracle_cfg.evm_private_key_env);
                return None;
            }
        };
        let key_bytes = hex::decode(key_hex.trim()).ok()?;
        let sk = K256SigningKey::from_slice(&key_bytes).ok()?;
        let addr = evm_address(&sk);
        info!(
            "[Relay] Watcher initialized. Signing address: {} — {} spoke(s): {}",
            addr,
            relay_cfg.spokes.len(),
            relay_cfg.spokes.iter().map(|s| s.label.as_str()).collect::<Vec<_>>().join(", ")
        );
        Some(Self { relay_cfg, oracle_cfg, signing_key: sk, evm_address: addr })
    }

    /// Background poll loop — runs indefinitely.
    pub async fn run(&self) {
        let poll_secs = self.relay_cfg.poll_interval_secs.max(10);
        loop {
            if let Err(e) = self.poll_all_spokes() {
                error!("[Relay] Poll error: {}", e);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(poll_secs)).await;
        }
    }

    fn load_state(&self) -> RelayPersist {
        match std::fs::read_to_string(&self.relay_cfg.state_path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => RelayPersist::default(),
        }
    }

    fn save_state(&self, state: &RelayPersist) {
        if let Ok(s) = serde_json::to_string_pretty(state) {
            if let Err(e) = std::fs::write(&self.relay_cfg.state_path, s) {
                warn!("[Relay] Failed to save state to {}: {}", self.relay_cfg.state_path, e);
            }
        }
    }

    fn poll_all_spokes(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut state = self.load_state();
        let topic0 = bridge_claim_requested_topic();
        let topic0_hex = format!("0x{}", hex::encode(topic0));

        for spoke in &self.relay_cfg.spokes {
            if let Err(e) = self.poll_spoke(spoke, &topic0_hex, &mut state) {
                warn!("[Relay] {} error: {}", spoke.label, e);
            }
        }

        Ok(())
    }

    fn poll_spoke(
        &self,
        spoke: &crate::config::RelaySpoke,
        topic0_hex: &str,
        state: &mut RelayPersist,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let from_block = state
            .last_scanned_block
            .get(&spoke.label)
            .copied()
            .unwrap_or(spoke.start_block);

        let latest_hex = rpc_call(&spoke.rpc, "eth_blockNumber", serde_json::json!([]))?;
        let latest_str = latest_hex.as_str().ok_or("blockNumber not a string")?;
        let latest_block = u64::from_str_radix(latest_str.trim_start_matches("0x"), 16)?;

        if from_block >= latest_block {
            debug!("[Relay] {} at block {} — nothing new", spoke.label, from_block);
            return Ok(());
        }

        // Use per-spoke log_chunk_size (default 100) to stay within public-RPC limits.
        let chunk = spoke.log_chunk_size.max(1);
        let to_block = (from_block + chunk).min(latest_block);

        let logs = rpc_call(
            &spoke.rpc,
            "eth_getLogs",
            serde_json::json!([{
                "fromBlock": format!("0x{:x}", from_block),
                "toBlock":   format!("0x{:x}", to_block),
                "address":   spoke.sender_address,
                "topics":    [topic0_hex],
            }]),
        )?;

        let log_arr = logs.as_array().cloned().unwrap_or_default();

        let mut any_failed = false;
        for log in &log_arr {
            if let Err(e) = self.process_log(log, &spoke.sender_address, state) {
                warn!("[Relay] {} log processing error: {}", spoke.label, e);
                any_failed = true;
            }
        }

        // Only advance scan position if all events in this chunk succeeded.
        // If any failed (e.g. mintForRelay reverted), keep last_scanned_block at from_block
        // so the failed event is retried on the next poll. Already-processed events are
        // skipped by the dedup_key check so they won't be double-submitted.
        if !any_failed {
            state.last_scanned_block.insert(spoke.label.clone(), to_block);
            self.save_state(state);
        }

        if !log_arr.is_empty() {
            info!(
                "[Relay] {} blocks {}-{}: {} BridgeClaimRequested event(s) processed{}",
                spoke.label,
                from_block,
                to_block,
                log_arr.len(),
                if any_failed { " (some failed — will retry)" } else { "" }
            );
        }

        Ok(())
    }

    fn process_log(
        &self,
        log: &serde_json::Value,
        sender_address: &str,
        state: &mut RelayPersist,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // topics[1]: indexed staker address — bytes32, last 20 bytes are the address
        let topics = log["topics"].as_array().ok_or("no topics array")?;
        if topics.len() < 2 {
            return Err("expected ≥2 topics (topic0 + indexed staker)".into());
        }
        let staker_topic = topics[1].as_str().ok_or("topics[1] not a string")?;
        let staker_raw = hex::decode(staker_topic.trim_start_matches("0x"))?;
        if staker_raw.len() != 32 {
            return Err(format!("topics[1] expected 32 bytes, got {}", staker_raw.len()).into());
        }
        let staker = format!("0x{}", hex::encode(&staker_raw[12..]));

        // data: amount (uint256, 32 bytes) ++ nonce (uint256, 32 bytes)
        let data_hex = log["data"].as_str().ok_or("no data field")?;
        let data_raw = hex::decode(data_hex.trim_start_matches("0x"))?;
        if data_raw.len() < 64 {
            return Err(format!("data too short: {} bytes", data_raw.len()).into());
        }
        let amount_slot: [u8; 32] = data_raw[0..32].try_into()?;
        let nonce_slot: [u8; 32] = data_raw[32..64].try_into()?;
        let nonce_hex = hex::encode(nonce_slot);

        let dedup_key = format!("{}:{}", sender_address.to_lowercase(), nonce_hex);
        if state.processed.contains(&dedup_key) {
            debug!("[Relay] Already processed: {}", dedup_key);
            return Ok(());
        }

        info!(
            "[Relay] Relay claim: staker={} nonce={} sender={}",
            staker, nonce_hex, sender_address
        );

        let calldata = encode_mint_for_relay(&staker, &amount_slot, sender_address, &nonce_slot)?;
        let evm_nonce = get_nonce(&self.oracle_cfg.evm_rpc, &self.evm_address)?;
        let gas_price = get_gas_price(&self.oracle_cfg.evm_rpc)?;
        let raw_tx = sign_legacy_tx(
            &self.signing_key,
            evm_nonce,
            gas_price,
            200_000,
            &self.oracle_cfg.reward_distributor,
            &calldata,
            self.oracle_cfg.evm_chain_id,
        )?;
        let tx_hash = send_raw_tx(&self.oracle_cfg.evm_rpc, &raw_tx)?;
        info!(
            "[Relay] mintForRelay submitted: tx={} staker={} nonce={}",
            tx_hash, staker, nonce_hex
        );

        // Wait up to ~15s for receipt to confirm success before marking processed.
        let mut confirmed = false;
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_secs(3));
            match check_tx_success(&self.oracle_cfg.evm_rpc, &tx_hash) {
                Some(true) => { confirmed = true; break; }
                Some(false) => {
                    warn!("[Relay] mintForRelay tx reverted: {} — will retry on next poll", tx_hash);
                    return Err(format!("mintForRelay reverted: {}", tx_hash).into());
                }
                None => continue, // not yet mined
            }
        }

        if confirmed {
            state.processed.insert(dedup_key);
            self.save_state(state);
        } else {
            // Not confirmed within timeout — treat as unprocessed so next poll retries.
            warn!("[Relay] mintForRelay tx not confirmed in time: {} — will retry", tx_hash);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_selector() {
        let sel = submit_legacy_claim_selector();
        assert_ne!(sel, [0x12, 0x34, 0x56, 0x78], "selector must not be placeholder");
        assert_eq!(sel.len(), 4);
    }

    #[test]
    fn test_abi_encoding_length() {
        let proof = crate::oracle::DormancyProof {
            version: "chrononode:dormancy:v1".to_string(),
            chain_id: "bitcoin".to_string(),
            address: "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".to_string(),
            dormant_since_block: 800_000,
            current_block: 850_000,
            threshold_blocks: 52_560,
            signer_pubkey: None,
            signature: None,
            evm_wallet: Some("0x201624cBa366250D08bCdA95e6eF64151687A447".to_string()),
            claim_type: 4,
            confidence_tier: 2,
            source_tx_hash: None,
            zk_proof: None,
            public_inputs: None,
        };
        let attestation = crate::oracle::OracleAttestation {
            proof,
            baals_pubkey: String::new(),
            baals_signature: "b".repeat(128),
            attested_at_block: 100,
            baals_block_hash: String::new(),
            attestation_id: String::new(),
            claim_id: None,
            evm_tx_hash: None,
            baals_tx_hash: None,
            proof_hash: Some("0x".to_string() + &"a".repeat(64)),
            status: "pending".to_string(),
            created_at: 1716600000,
            submitted_at: None,
            error: None,
        };
        let calldata = encode_submit_legacy_claim(
            &attestation,
            "0x201624cBa366250D08bCdA95e6eF64151687A447",
        )
        .unwrap();
        // 4 (selector) + 11*32 (struct) + 32 (offset) + 32 (length) + 64 (sig) = 484
        assert_eq!(calldata.len(), 484, "unexpected calldata length: {}", calldata.len());
    }

    #[test]
    fn test_rlp_uint() {
        assert_eq!(rlp_encode_uint(0), vec![0x80]);
        assert_eq!(rlp_encode_uint(1), vec![0x01]);
        assert_eq!(rlp_encode_uint(0x7f), vec![0x7f]);
        assert_eq!(rlp_encode_uint(0x80), vec![0x81, 0x80]);
        assert_eq!(rlp_encode_uint(256), vec![0x82, 0x01, 0x00]);
    }

    #[test]
    fn test_submission_state_retry() {
        let mut state = SubmissionState {
            chain_id: "bitcoin".to_string(),
            address: "1abc".to_string(),
            submitted_at_block: 100,
            evm_tx_hash: None,
            status: "pending".to_string(),
            attempts: 0,
            last_attempt_time: unix_now(),
        };
        assert!(state.is_eligible_for_retry());

        state.attempts = 1;
        // just submitted, backoff 2 min
        assert!(!state.is_eligible_for_retry());

        state.status = "confirmed".to_string();
        assert!(!state.is_eligible_for_retry());
    }

    #[test]
    fn test_evm_address_derivation() {
        // Known key → known address
        let key_bytes = hex::decode(
            "76cf1b0bff9468e5d60b78b4f341dd1934868447975e4af057781dea14a01c04",
        )
        .unwrap();
        let sk = K256SigningKey::from_slice(&key_bytes).unwrap();
        let addr = evm_address(&sk);
        // Expected: 0x201624cBa366250D08bCdA95e6eF64151687A447 (lowercase comparison)
        assert_eq!(
            addr.to_lowercase(),
            "0x201624cba366250d08bcda95e6ef64151687a447"
        );
    }

    // ── Phase L relay tests ───────────────────────────────────────────────────

    #[test]
    fn test_bridge_claim_requested_topic() {
        let topic = bridge_claim_requested_topic();
        let hex = format!("0x{}", hex::encode(topic));
        println!("BridgeClaimRequested topic: {}", hex);
        // Verify against on-chain confirmed value
        assert_eq!(hex, "0x093b86e9ff45f8c6748611e4f8d284abde89dc7f4c010451d9138d319c4fd9d5");
    }

    #[test]
    fn test_mint_for_relay_selector() {
        let sel = mint_for_relay_selector();
        assert_eq!(sel.len(), 4);
        assert_ne!(sel, [0u8; 4]);
        // Must differ from submitLegacyClaim selector
        assert_ne!(sel, submit_legacy_claim_selector());
    }

    #[test]
    fn test_encode_mint_for_relay_length() {
        let mut amount = [0u8; 32];
        amount[24..].copy_from_slice(&500u64.to_be_bytes());
        let mut nonce = [0u8; 32];
        // nonce = 0

        let calldata = encode_mint_for_relay(
            "0x201624cBa366250D08bCdA95e6eF64151687A447",
            &amount,
            "0x3F1E0400fb8f19FeFA8aA6B8d23468949E73a7B5",
            &nonce,
        )
        .unwrap();
        // 4 (selector) + 4 × 32 (fixed args) = 132 bytes
        assert_eq!(calldata.len(), 132, "unexpected calldata length: {}", calldata.len());
        // First 4 bytes are selector
        assert_eq!(&calldata[..4], &mint_for_relay_selector());
    }

    #[test]
    fn test_encode_mint_for_relay_user_position() {
        let mut amount = [0u8; 32];
        amount[31] = 1; // amount = 1
        let nonce = [0u8; 32];
        let user = "0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF";

        let calldata = encode_mint_for_relay(user, &amount, "0x1234567890123456789012345678901234567890", &nonce).unwrap();
        // user slot is at bytes 4..36 (after selector); last 20 bytes are address
        let user_bytes = hex::decode("DeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF").unwrap();
        assert_eq!(&calldata[4..16], &[0u8; 12]); // 12-byte left-pad
        assert_eq!(&calldata[16..36], user_bytes.as_slice());
    }

    #[test]
    fn test_relay_persist_dedup() {
        let mut state = RelayPersist::default();
        let key = "0xsender:0000000000000000000000000000000000000000000000000000000000000000".to_string();
        assert!(!state.processed.contains(&key));
        state.processed.insert(key.clone());
        assert!(state.processed.contains(&key));
        // Different nonce — not present
        let key2 = "0xsender:0000000000000000000000000000000000000000000000000000000000000001".to_string();
        assert!(!state.processed.contains(&key2));
    }
}
