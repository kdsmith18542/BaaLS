use clap::Subcommand;
use std::path::PathBuf;

use crate::{
    config::{Config, StorageBackend},
    AnyStorage, ContractId, RedbStorage, SledStorage, Storage,
};

use crate::cli::{parse_pubkey, text_or_json};

#[derive(Subcommand)]
pub enum ProofCommands {
    Account {
        address: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Contract {
        contract_id: String,
        key: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Verify {
        proof_file: PathBuf,
    },
}

pub fn handle_proof(
    action: ProofCommands,
    json: bool,
    backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        ProofCommands::Account { address, data_dir } => {
            let pk = parse_pubkey(&address)?;
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let all_accounts = storage.get_all_accounts()?;
            let mut smt = crate::SparseMerkleTree::new();
            for (addr, acct) in &all_accounts {
                let bytes = bincode::serialize(acct)?;
                smt.insert(addr.to_bytes(), bytes);
            }
            let proof = smt.generate_proof(pk.to_bytes());
            Ok(text_or_json(
                json,
                &format!(
                    "Proof root: {}\nProof entries: {}",
                    hex::encode(proof.root),
                    proof.proof.len()
                ),
                serde_json::json!({
                    "root": hex::encode(proof.root),
                    "proof": proof.proof.iter().map(hex::encode).collect::<Vec<_>>(),
                    "key": hex::encode(proof.key),
                    "value_hex": hex::encode(&proof.value),
                }),
            ))
        }
        ProofCommands::Contract { contract_id, key, data_dir } => {
            let cid_bytes = hex::decode(&contract_id)?;
            if cid_bytes.len() != 32 {
                return Err("Contract ID must be 32 bytes hex".into());
            }
            let mut cid_arr = [0u8; 32];
            cid_arr.copy_from_slice(&cid_bytes);
            let cid = ContractId::from_bytes(&cid_arr);
            let key_bytes = hex::decode(&key)?;
            let mut key_arr = [0u8; 32];
            let len = key_bytes.len().min(32);
            key_arr[..len].copy_from_slice(&key_bytes[..len]);
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let all_storage = storage.contract_storage_read_all(&cid)?;
            let mut smt = crate::SparseMerkleTree::new();
            for (k, v) in all_storage {
                let mut k_arr = [0u8; 32];
                let kl = k.len().min(32);
                k_arr[..kl].copy_from_slice(&k[..kl]);
                smt.insert(k_arr, v);
            }
            let proof = smt.generate_proof(key_arr);
            Ok(text_or_json(
                json,
                &format!(
                    "Contract proof root: {}\nProof entries: {}",
                    hex::encode(proof.root),
                    proof.proof.len()
                ),
                serde_json::json!({
                    "root": hex::encode(proof.root),
                    "proof": proof.proof.iter().map(hex::encode).collect::<Vec<_>>(),
                    "key": hex::encode(proof.key),
                    "value_hex": hex::encode(&proof.value),
                }),
            ))
        }
        ProofCommands::Verify { proof_file } => {
            let data = std::fs::read_to_string(&proof_file)?;
            let proof: crate::SparseMerkleProof = serde_json::from_str(&data)?;
            let valid =
                crate::SparseMerkleTree::verify_proof(proof.key, &proof.value, &proof, proof.root);
            Ok(text_or_json(
                json,
                if valid { "Proof VALID" } else { "Proof INVALID" },
                serde_json::json!({"valid": valid}),
            ))
        }
    }
}
