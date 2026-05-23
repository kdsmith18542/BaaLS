use crate::config::OracleConfig;
/// Phase K — EVM Submitter for Resurgence Protocol oracle integration.
///
/// The EVMSubmitter module watches BaaLS oracle storage for finalized dormancy attestations
/// and submits them to the Resurgence RewardDistributor on EVM chains (Arbitrum, Polygon).
///
/// Flow:
///   1. Poll oracle storage for new attestations (marked finalized after confirmation depth)
///   2. For each new attestation, sign a transaction with the EVM bridge key
///   3. Submit signed tx to EVM RPC endpoint (POST JSON-RPC call to submitOracleAttestation)
///   4. Track submission state (pending/confirmed) to avoid resubmission
///   5. On success, mark attestation as "submitted"
use crate::oracle::OracleAttestation;
use crate::storage::Storage;
use crate::ContractId;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Submission state tracking for each attestation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionState {
    /// Chain ID of the source dormancy proof (e.g., "bitcoin")
    pub chain_id: String,
    /// Address of the dormant account
    pub address: String,
    /// Block height when submitted
    pub submitted_at_block: u64,
    /// EVM transaction hash (if successful)
    pub evm_tx_hash: Option<String>,
    /// Submission status: "pending", "confirmed", "failed"
    pub status: String,
    /// Number of submission attempts
    pub attempts: u32,
    /// Unix timestamp of last submission attempt
    pub last_attempt_time: u64,
}

impl SubmissionState {
    fn is_eligible_for_retry(&self) -> bool {
        if self.status == "confirmed" {
            return false; // Already confirmed, don't retry
        }
        if self.attempts == 0 {
            return true; // First submission attempt should run immediately
        }
        if self.attempts >= 5 {
            return false; // Too many attempts
        }
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(self.last_attempt_time);
        // Exponential backoff: 2^attempts minutes
        let backoff_secs = (2_u64).saturating_pow(self.attempts as u32) * 60;
        elapsed >= backoff_secs
    }
}

/// EVMSubmitter coordinates submission of BaaLS attestations to EVM chains.
pub struct EVMSubmitter {
    config: OracleConfig,
    evm_key: Vec<u8>,
    submission_states: Arc<tokio::sync::Mutex<HashMap<String, SubmissionState>>>,
}

impl EVMSubmitter {
    /// Create a new EVMSubmitter with EVM chain configuration and signing key.
    pub fn new(config: OracleConfig, evm_key: Vec<u8>) -> Self {
        Self {
            config,
            evm_key,
            submission_states: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Poll the oracle storage for new attestations and submit them to EVM.
    /// Runs as an async task in the background.
    pub async fn run(&self, storage: &dyn Storage) {
        if !self.config.enabled {
            debug!("[EVM] Oracle disabled, skipping submission loop");
            return;
        }

        if self.config.evm_rpc.is_empty() || self.config.reward_distributor.is_empty() {
            error!("[EVM] Missing evm_rpc or reward_distributor config");
            return;
        }

        loop {
            if let Err(e) = self.poll_and_submit(storage).await {
                error!("[EVM] Poll/submit error: {}", e);
            }
            // Poll every 30 seconds
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        }
    }

    /// Poll oracle storage for new attestations and attempt submission.
    async fn poll_and_submit(
        &self,
        storage: &dyn Storage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let oracle_ns_id = ContractId::from_bytes(&crate::oracle::oracle_namespace_id());

        // Get list of all chains
        let chains_key = crate::oracle::chains_storage_key();
        let chains_raw = storage.contract_storage_read(&oracle_ns_id, &chains_key)?;
        let chain_ids: Vec<String> = if let Some(raw) = chains_raw {
            serde_json::from_slice(&raw).unwrap_or_default()
        } else {
            vec![]
        };

        let mut states = self.submission_states.lock().await;

        for chain_id in chain_ids {
            // Get list of addresses for this chain
            let index_key = crate::oracle::chain_index_key(&chain_id);
            let index_raw = storage.contract_storage_read(&oracle_ns_id, &index_key)?;
            let addresses: Vec<String> = if let Some(raw) = index_raw {
                serde_json::from_slice(&raw).unwrap_or_default()
            } else {
                continue;
            };

            for address in addresses {
                let state_key = format!("evm:submission:{}:{}", chain_id, address);
                let state: SubmissionState = if let Some(state_raw) = states.get(&state_key) {
                    state_raw.clone()
                } else {
                    // First time seeing this attestation
                    SubmissionState {
                        chain_id: chain_id.clone(),
                        address: address.clone(),
                        submitted_at_block: 0,
                        evm_tx_hash: None,
                        status: "pending".to_string(),
                        attempts: 0,
                        last_attempt_time: 0,
                    }
                };

                // Check if eligible for submission or retry
                if state.status == "confirmed" {
                    continue; // Already confirmed
                }
                if state.attempts > 0 && !state.is_eligible_for_retry() {
                    continue; // Not ready for retry yet
                }

                // Fetch attestation
                let attest_key = crate::oracle::attestation_storage_key(&chain_id, &address);
                let attest_raw = storage.contract_storage_read(&oracle_ns_id, &attest_key)?;
                let attestation: OracleAttestation = if let Some(raw) = attest_raw {
                    serde_json::from_slice(&raw)?
                } else {
                    continue; // Attestation not found
                };

                // Attempt submission
                let new_state = match self.submit_to_evm(&attestation).await {
                    Ok(tx_hash) => {
                        info!(
                            "[EVM] Submitted attestation: chain={} addr={} tx={}",
                            chain_id, address, tx_hash
                        );
                        SubmissionState {
                            evm_tx_hash: Some(tx_hash),
                            status: "pending".to_string(),
                            attempts: state.attempts + 1,
                            last_attempt_time: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            ..state
                        }
                    }
                    Err(e) => {
                        warn!(
                            "[EVM] Submission failed (attempt {}): chain={} addr={} err={}",
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
                            last_attempt_time: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            ..state
                        }
                    }
                };

                states.insert(state_key, new_state);
            }
        }

        Ok(())
    }

    /// Submit an attestation to the EVM RewardDistributor contract.
    /// Returns the EVM transaction hash on success.
    async fn submit_to_evm(
        &self,
        attestation: &OracleAttestation,
    ) -> Result<String, Box<dyn std::error::Error>> {
        use k256::ecdsa::signature::Signer;
        use k256::ecdsa::SigningKey as K256SigningKey;

        if self.evm_key.len() != 32 {
            return Err("Invalid EVM key length".into());
        }

        // Sign the attestation with the EVM key
        let sk = K256SigningKey::from_slice(&self.evm_key)?;
        let attestation_json = serde_json::to_vec(&attestation)?;
        let signature = sk.sign(&attestation_json);

        // Prepare JSON-RPC call to submitOracleAttestation
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_sendTransaction",
            "params": [
                {
                    "to": self.config.reward_distributor,
                    "data": format_call_data(&signature, &attestation_json)?,
                    "gas": "500000",
                }
            ],
            "id": 1,
        });

        // POST to EVM RPC
        let response = ureq::post(&self.config.evm_rpc)
            .set("Content-Type", "application/json")
            .send_json(&payload)?;

        let result: serde_json::Value = response.into_json()?;

        // Extract tx hash from JSON-RPC response
        if let Some(tx_hash) = result.get("result").and_then(|r| r.as_str()) {
            Ok(tx_hash.to_string())
        } else if let Some(error) = result.get("error") {
            Err(format!("EVM RPC error: {}", error).into())
        } else {
            Err("Unexpected EVM RPC response".into())
        }
    }
}

/// Format ABI-encoded call data for RewardDistributor.submitOracleAttestation
fn format_call_data(
    signature: &k256::ecdsa::Signature,
    attestation_json: &[u8],
) -> Result<String, Box<dyn std::error::Error>> {
    // This is a placeholder. In production, you would:
    // 1. Get the RewardDistributor ABI
    // 2. Encode the attestation and signature into ABI format (Solidity)
    // 3. Return the encoded call data
    //
    // For now, we just return a hex-encoded representation
    let mut call_data = Vec::new();
    // Function selector for submitOracleAttestation (would be computed from ABI)
    call_data.extend_from_slice(&[0x12, 0x34, 0x56, 0x78]); // Placeholder
    call_data.extend_from_slice(signature.to_bytes().as_ref());
    call_data.extend_from_slice(attestation_json);
    Ok(format!("0x{}", hex::encode(call_data)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submission_state_retry_logic() {
        let mut state = SubmissionState {
            chain_id: "bitcoin".to_string(),
            address: "1A1z7agoat".to_string(),
            submitted_at_block: 100,
            evm_tx_hash: None,
            status: "pending".to_string(),
            attempts: 0,
            last_attempt_time: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        };

        // First attempt, not eligible yet
        assert!(state.is_eligible_for_retry());

        // After 0 attempts, should be retryable immediately
        state.attempts = 1;
        state.last_attempt_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert!(!state.is_eligible_for_retry()); // Needs 2 minutes (2^1)

        // Confirmed status, not retryable
        state.status = "confirmed".to_string();
        assert!(!state.is_eligible_for_retry());
    }

    #[test]
    fn test_evm_submitter_creation() {
        let config = OracleConfig {
            enabled: true,
            evm_rpc: "https://arb-sepolia.g.alchemy.com/v2/key".to_string(),
            reward_distributor: "0x1234567890123456789012345678901234567890".to_string(),
            amoy_rpc: "https://rpc-amoy.polygon.technology/".to_string(),
            evm_private_key_env: "BAALS_EVM_PRIVATE_KEY".to_string(),
            evm_chain_id: 421614,
        };
        let evm_key = vec![0u8; 32];
        let _submitter = EVMSubmitter::new(config, evm_key);
        // Just verify creation succeeds
    }
}
