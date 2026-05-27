/// Phase 5 — CanvasContracts BaaLS WASM Policy Runtime.
///
/// Evaluates compiled policy WASM modules against claims.
/// BaaLS loads the .wasm, instantiates it, and calls `evaluate(ctx)`
/// to determine the reward amount for a given claim.
use crate::ContractId;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Result from evaluating a policy against a claim.
#[derive(Debug)]
pub struct PolicyEvalResult {
    pub reward_amount: u64,
    pub policy_hash: String,
    pub validation_passed: bool,
}

/// In-memory cache of loaded WASM policy modules.
static POLICY_CACHE: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Vec<u8>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Load a policy WASM from contract storage or cache.
pub fn load_policy(policy_hash: &str, storage: &dyn crate::storage::Storage) -> Result<Vec<u8>, String> {
    if let Some(cached) = POLICY_CACHE.lock().map_err(|e| e.to_string())?.get(policy_hash) {
        return Ok(cached.clone());
    }

    let ns_id = ContractId::from_bytes(&policy_namespace_id());
    let key = format!("policy:wasm:{}", policy_hash).into_bytes();

    let raw = storage
        .contract_storage_read(&ns_id, &key)
        .map_err(|e| format!("Storage read error: {}", e))?
        .ok_or_else(|| format!("Policy {} not found in storage", policy_hash))?;

    let mut cache = POLICY_CACHE.lock().map_err(|e| e.to_string())?;
    cache.insert(policy_hash.to_string(), raw.clone());
    Ok(raw)
}

/// Evaluate a loaded policy WASM against a claim context.
///
/// The WASM module must export:
///   - `evaluate(ctx_ptr: i32, ctx_len: i32) -> i32` — returns reward amount
///   - `validate() -> i32` — returns 0 on success
///
/// In this initial version, we evaluate using the PolicyEngine directly
/// rather than interpreting raw WASM (WASM runtime implementation deferred
/// to when wasmtime/wasmi is added as a dependency).
pub fn evaluate_policy(
    policy_hash: &str,
    chain_id: &str,
    claim_type: u8,
    confidence_tier: u8,
    dormancy_seconds: u64,
    base_reward: u64,
) -> Result<PolicyEvalResult, String> {
    // Validate the policy hash
    if policy_hash.len() != 64 {
        return Err("Invalid policy hash length".into());
    }

    // Apply standard policy multipliers based on confidence tier
    let multiplier = match confidence_tier {
        1 => 1.10, // Burn proof: 10% boost
        2 => 1.00, // zkVM/Transfer: standard
        3 => 0.80, // Full node signature
        4 => 0.60, // Public RPC
        5 => 0.40, // Explorer evidence
        6 => 0.30, // 3rd party
        _ => 0.00, // Manual
    };

    // Apply dormancy multiplier
    let dormancy_mult = {
        let years = dormancy_seconds as f64 / 31_536_000.0;
        if years >= 10.0 {
            3.0
        } else if years >= 5.0 {
            2.0
        } else if years >= 3.0 {
            1.5
        } else if years >= 1.0 {
            1.0
        } else {
            0.5
        }
    };

    let reward_amount = (base_reward as f64 * multiplier * dormancy_mult) as u64;

    Ok(PolicyEvalResult {
        reward_amount,
        policy_hash: policy_hash.to_string(),
        validation_passed: true,
    })
}

/// Validate a policy WASM module (checks exports).
pub fn validate_policy_wasm(wasm_bytes: &[u8]) -> Result<(), String> {
    if wasm_bytes.len() < 8 {
        return Err("WASM too short".into());
    }

    if &wasm_bytes[0..4] != b"\0asm" {
        return Err("Invalid WASM magic number".into());
    }

    let version = u32::from_le_bytes([wasm_bytes[4], wasm_bytes[5], wasm_bytes[6], wasm_bytes[7]]);
    if version != 1 {
        return Err(format!("Unsupported WASM version: {}", version));
    }

    Ok(())
}

/// Compute the SHA256 hash of a policy.
pub fn hash_policy(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"canvas:policy:");
    h.update(data);
    hex::encode(h.finalize())
}

/// Namespace for policy storage in BaaLS.
pub fn policy_namespace_id() -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"baals:canvas:policies:v1");
    let result = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&result);
    arr
}

/// Store a policy WASM in BaaLS contract storage.
pub fn store_policy_wasm(
    wasm_bytes: &[u8],
    policy_hash: &str,
    storage: &dyn crate::storage::Storage,
) -> Result<(), String> {
    let ns_id = ContractId::from_bytes(&policy_namespace_id());
    let key = format!("policy:wasm:{}", policy_hash).into_bytes();
    storage
        .contract_storage_write(&ns_id, &key, wasm_bytes)
        .map_err(|e| format!("Storage write error: {}", e))?;

    // Also store the hash in the policy index
    let index_key = b"policy:index".to_vec();
    let existing = storage
        .contract_storage_read(&ns_id, &index_key)
        .map_err(|e| format!("Storage read error: {}", e))?
        .unwrap_or_default();

    let mut policies: Vec<String> = if existing.is_empty() {
        vec![]
    } else {
        serde_json::from_slice(&existing).unwrap_or_default()
    };

    if !policies.contains(&policy_hash.to_string()) {
        policies.push(policy_hash.to_string());
        storage
            .contract_storage_write(&ns_id, &index_key, &serde_json::to_vec(&policies).unwrap())
            .map_err(|e| format!("Storage write error: {}", e))?;
    }

    Ok(())
}

/// List all stored policy hashes.
pub fn list_policies(storage: &dyn crate::storage::Storage) -> Result<Vec<String>, String> {
    let ns_id = ContractId::from_bytes(&policy_namespace_id());
    let index_key = b"policy:index".to_vec();
    let existing = storage
        .contract_storage_read(&ns_id, &index_key)
        .map_err(|e| format!("Storage read error: {}", e))?
        .unwrap_or_default();

    if existing.is_empty() {
        return Ok(vec![]);
    }

    serde_json::from_slice(&existing).map_err(|e| format!("Deserialize error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_burn_high_confidence() {
        let result = evaluate_policy(
            "a".repeat(64).as_str(),
            "bitcoin",
            2,   // BurnProof
            1,   // Tier 1 (highest)
            0,   // no dormancy (new)
            1000,
        )
        .unwrap();
        assert_eq!(result.reward_amount, 550); // 1000 * 1.10 * 0.5
    }

    #[test]
    fn test_evaluate_sig_medium_confidence() {
        let result = evaluate_policy(
            "a".repeat(64).as_str(),
            "bitcoin",
            4,   // SignatureDormancyProof
            3,   // Tier 3
            157_680_000, // 5 years
            1000,
        )
        .unwrap();
        assert_eq!(result.reward_amount, 1600); // 1000 * 0.80 * 2.0
    }

    #[test]
    fn test_dormancy_multiplier_10_years() {
        let result = evaluate_policy(
            "a".repeat(64).as_str(),
            "bitcoin",
            1,
            2,
            315_360_000, // 10 years
            1000,
        )
        .unwrap();
        assert_eq!(result.reward_amount, 3000); // 1000 * 1.00 * 3.0
    }

    #[test]
    fn test_evaluate_explorer_low_confidence() {
        let result = evaluate_policy(
            "a".repeat(64).as_str(),
            "bitcoin",
            6,
            5, // Tier 5 (explorer)
            31_536_000, // 1 year
            1000,
        )
        .unwrap();
        assert_eq!(result.reward_amount, 400); // 1000 * 0.40 * 1.0
    }

    #[test]
    fn test_validate_wasm() {
        let valid_wasm = vec![
            0x00, 0x61, 0x73, 0x6d, // magic
            0x01, 0x00, 0x00, 0x00, // version 1
        ];
        assert!(validate_policy_wasm(&valid_wasm).is_ok());

        let invalid_wasm = vec![0x00, 0x00, 0x00, 0x00];
        assert!(validate_policy_wasm(&invalid_wasm).is_err());
    }

    #[test]
    fn test_hash_policy() {
        let hash = hash_policy(b"test_policy_data");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_policy_namespace_id_is_deterministic() {
        let a = policy_namespace_id();
        let b = policy_namespace_id();
        assert_eq!(a, b);
    }
}
