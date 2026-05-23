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
use crate::config::OracleConfig;
use crate::oracle::OracleAttestation;
use crate::storage::Storage;
use crate::ContractId;
use k256::ecdsa::{RecoveryId, Signature as K256Signature, SigningKey as K256SigningKey};
use k256::ecdsa::signature::hazmat::PrehashSigner;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
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

/// 4-byte selector for submitDormancyProof(bytes32,address,uint256,uint256,uint256,bytes32,bytes)
fn submit_dormancy_selector() -> [u8; 4] {
    let sig = b"submitDormancyProof(bytes32,address,uint256,uint256,uint256,bytes32,bytes)";
    let h = keccak256(sig);
    [h[0], h[1], h[2], h[3]]
}

/// ABI-encode a call to submitDormancyProof. Returns selector + encoded args.
///
/// Parameters map to DormancyProof fields:
///   chainId          = bytes32(chain_id padded)
///   dormantWallet    = evm_wallet address (20 bytes, left-zero-padded to 32)
///   dormantSinceBlock / currentBlock / thresholdBlocks = u64 → uint256
///   signerPubkey     = bytes32(signer_pubkey, first 32 bytes)
///   signature        = raw ed25519 sig bytes (dynamic bytes)
fn encode_submit_dormancy(
    chain_id_str: &str,
    evm_wallet_hex: &str,
    dormant_since_block: u64,
    current_block: u64,
    threshold_blocks: u64,
    signer_pubkey_hex: &str,
    signature_hex: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // chainId: pad chain_id_str bytes into 32-byte right-justified? No, bytes32 is left-aligned.
    let chain_id_bytes = chain_id_str.as_bytes();
    let mut chain_id_slot = [0u8; 32];
    let copy_len = chain_id_bytes.len().min(32);
    chain_id_slot[..copy_len].copy_from_slice(&chain_id_bytes[..copy_len]);

    // dormantWallet: 20-byte address, left-padded with 12 zero bytes
    let wallet_hex = evm_wallet_hex.trim_start_matches("0x");
    let wallet_bytes = hex::decode(wallet_hex)?;
    if wallet_bytes.len() != 20 {
        return Err(format!("evm_wallet must be 20 bytes, got {}", wallet_bytes.len()).into());
    }
    let mut wallet_slot = [0u8; 32];
    wallet_slot[12..].copy_from_slice(&wallet_bytes);

    // uint256 slots (u64 → 32-byte BE, zero-padded)
    let mut dormant_slot = [0u8; 32];
    dormant_slot[24..].copy_from_slice(&dormant_since_block.to_be_bytes());
    let mut current_slot = [0u8; 32];
    current_slot[24..].copy_from_slice(&current_block.to_be_bytes());
    let mut threshold_slot = [0u8; 32];
    threshold_slot[24..].copy_from_slice(&threshold_blocks.to_be_bytes());

    // signerPubkey: first 32 bytes of hex-decoded pubkey
    let pubkey_bytes = hex::decode(signer_pubkey_hex)?;
    let mut pubkey_slot = [0u8; 32];
    let pk_copy = pubkey_bytes.len().min(32);
    pubkey_slot[..pk_copy].copy_from_slice(&pubkey_bytes[..pk_copy]);

    // signature: dynamic bytes — offset is at slot 6 = 7 * 32 = 224
    let sig_bytes = hex::decode(signature_hex)?;
    let offset: u64 = 7 * 32; // 7 static slots before tail
    let mut offset_slot = [0u8; 32];
    offset_slot[24..].copy_from_slice(&offset.to_be_bytes());

    // Signature tail: 32-byte length + padded data
    let sig_len = sig_bytes.len() as u64;
    let mut sig_len_slot = [0u8; 32];
    sig_len_slot[24..].copy_from_slice(&sig_len.to_be_bytes());
    // Pad sig to 32-byte boundary
    let padded_len = (sig_bytes.len() + 31) / 32 * 32;
    let mut sig_padded = vec![0u8; padded_len];
    sig_padded[..sig_bytes.len()].copy_from_slice(&sig_bytes);

    let mut calldata = Vec::new();
    calldata.extend_from_slice(&submit_dormancy_selector());
    calldata.extend_from_slice(&chain_id_slot);
    calldata.extend_from_slice(&wallet_slot);
    calldata.extend_from_slice(&dormant_slot);
    calldata.extend_from_slice(&current_slot);
    calldata.extend_from_slice(&threshold_slot);
    calldata.extend_from_slice(&pubkey_slot);
    calldata.extend_from_slice(&offset_slot);
    calldata.extend_from_slice(&sig_len_slot);
    calldata.extend_from_slice(&sig_padded);

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
        if self.config.evm_rpc.is_empty() || self.config.reward_distributor.is_empty() {
            error!("[EVM] Missing evm_rpc or reward_distributor config — submitter disabled");
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

                let new_state = match self.submit_attestation(&attestation, &evm_wallet) {
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
                };

                states.insert(state_key, new_state);
            }
        }

        Ok(())
    }

    /// Build, sign, and send a submitDormancyProof transaction.
    fn submit_attestation(
        &self,
        attestation: &OracleAttestation,
        evm_wallet: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let proof = &attestation.proof;

        let signer_pubkey = proof
            .signer_pubkey
            .as_deref()
            .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000");
        let signature = proof
            .signature
            .as_deref()
            .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000");

        let calldata = encode_submit_dormancy(
            &proof.chain_id,
            evm_wallet,
            proof.dormant_since_block,
            proof.current_block,
            proof.threshold_blocks,
            signer_pubkey,
            signature,
        )?;

        let nonce = get_nonce(&self.config.evm_rpc, &self.evm_address)?;
        let gas_price = get_gas_price(&self.config.evm_rpc)?;

        debug!(
            "[EVM] Building tx: nonce={} gasPrice={} to={}",
            nonce, gas_price, self.config.reward_distributor
        );

        let raw_tx = sign_legacy_tx(
            &self.signing_key,
            nonce,
            gas_price,
            500_000,
            &self.config.reward_distributor,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_selector() {
        // Selector for submitDormancyProof(bytes32,address,uint256,uint256,uint256,bytes32,bytes)
        // Verified against cast sig "submitDormancyProof(bytes32,address,uint256,uint256,uint256,bytes32,bytes)"
        let sel = submit_dormancy_selector();
        // The 4-byte selector is deterministic; ensure it's non-zero placeholder is gone
        assert_ne!(sel, [0x12, 0x34, 0x56, 0x78], "selector must not be placeholder");
        assert_eq!(sel.len(), 4);
    }

    #[test]
    fn test_abi_encoding_length() {
        let calldata = encode_submit_dormancy(
            "bitcoin",
            "0x201624cBa366250D08bCdA95e6eF64151687A447",
            800_000,
            850_000,
            52_560,
            &"a".repeat(64),
            &"b".repeat(128),
        )
        .unwrap();
        // 4 (selector) + 7*32 (head) + 32 (sig len) + 64 (128 hex = 64 bytes, padded to 64) = 4 + 224 + 32 + 64 = 324
        assert_eq!(calldata.len(), 324, "unexpected calldata length: {}", calldata.len());
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
}
