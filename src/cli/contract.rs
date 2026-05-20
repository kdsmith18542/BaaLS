use clap::Subcommand;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::text_or_json;

#[derive(Subcommand)]
pub enum ContractCommands {
    Inspect {
        #[arg(short, long)]
        contract_id: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Simulate {
        #[arg(short, long)]
        wasm: PathBuf,
        #[arg(short, long)]
        method: String,
        #[arg(short, long)]
        args: Option<String>,
        #[arg(short = 'g', long, default_value = "1000000")]
        gas_limit: u64,
    },
    EstimateGas {
        #[arg(short, long)]
        contract_id: String,
        #[arg(short, long)]
        method: String,
        #[arg(short, long)]
        args: Option<String>,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Abi {
        #[arg(short, long)]
        contract_id: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    VerifyWasm {
        #[arg(short, long)]
        wasm: PathBuf,
    },
}

fn parse_contract_args(
    raw_args: Option<String>,
) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error>> {
    let Some(raw) = raw_args else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let to_bytes = |value: serde_json::Value| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        match value {
            serde_json::Value::String(s) => Ok(s.into_bytes()),
            other => Ok(serde_json::to_vec(&other)?),
        }
    };

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Array(items)) => items.into_iter().map(to_bytes).collect(),
        Ok(value) => Ok(vec![to_bytes(value)?]),
        Err(_) => Ok(vec![raw.into_bytes()]),
    }
}

pub fn handle_contract(
    action: ContractCommands,
    json: bool,
    _backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        ContractCommands::Inspect { contract_id, data_dir } => {
            let contract_id_bytes =
                hex::decode(&contract_id).unwrap_or_else(|_| contract_id.as_bytes().to_vec());
            let contract_id_arr: [u8; 32] = if contract_id_bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&contract_id_bytes);
                arr
            } else {
                let mut hasher = Sha256::new();
                Digest::update(&mut hasher, &contract_id_bytes);
                let result = hasher.finalize();
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&result[..]);
                arr
            };

            let contracts_dir = data_dir.join("contracts");
            if contracts_dir.exists() {
                let contract_file =
                    contracts_dir.join(format!("{}.wasm", hex::encode(&contract_id_arr[..8])));
                if contract_file.exists() {
                    let code = std::fs::read(&contract_file)?;
                    let mut hasher = Sha256::new();
                    Digest::update(&mut hasher, &code);
                    Ok(text_or_json(
                        json,
                        &format!(
                            "Contract: {}\nCode size: {} bytes\nType: WASM",
                            contract_id,
                            code.len()
                        ),
                        serde_json::json!({
                            "contract_id": contract_id,
                            "exists": true,
                            "code_size": code.len(),
                            "code_hash": hex::encode(hasher.finalize())
                        }),
                    ))
                } else {
                    Ok(text_or_json(
                        json,
                        &format!("Contract not found: {}", contract_id),
                        serde_json::json!({"contract_id": contract_id, "exists": false}),
                    ))
                }
            } else {
                Ok(text_or_json(
                    json,
                    &format!("Contract not found: {}", contract_id),
                    serde_json::json!({"contract_id": contract_id, "exists": false}),
                ))
            }
        }
        ContractCommands::Simulate { wasm, method, args, gas_limit } => {
            let wasm_data = std::fs::read(&wasm)?;

            if !wasm_data.starts_with(b"\0asm") {
                return Err("Invalid WASM magic number".into());
            }

            let arg_bytes = parse_contract_args(args.clone())?;
            let mut seed = [0u8; 32];
            rand::rng().fill_bytes(&mut seed);
            let caller = crate::PublicKey::from(
                ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key(),
            );

            let mut contract_hash = [0u8; 32];
            contract_hash.copy_from_slice(&Sha256::digest(&wasm_data));
            let contract_id = crate::ContractId::from_bytes(&contract_hash);

            let temp_dir = tempfile::TempDir::new()?;
            let storage = crate::SledStorage::new(temp_dir.path())?;
            let engine = crate::BaaLSContractEngine::new(storage.clone())?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

            match engine.execute_wasm_contract(
                &wasm_data,
                &method,
                &arg_bytes,
                &caller,
                &contract_id,
                &storage,
                true,
                gas_limit,
                0,
                now,
            ) {
                Ok((output, gas_used, side_effects)) => {
                    let logs = side_effects
                        .events
                        .iter()
                        .map(|(topic, data)| {
                            serde_json::json!({
                                "topic_hex": hex::encode(topic),
                                "data_hex": hex::encode(data),
                            })
                        })
                        .collect::<Vec<_>>();
                    Ok(text_or_json(
                        json,
                        &format!(
                            "Contract simulation succeeded\nWASM: {}\nMethod: {}\nArgs: {}\nGas used: {}\nOutput bytes: {}",
                            wasm.display(),
                            method,
                            arg_bytes.len(),
                            gas_used,
                            output.len()
                        ),
                        serde_json::json!({
                            "wasm": wasm.to_string_lossy(),
                            "method": method,
                            "args": args,
                            "code_size": wasm_data.len(),
                            "status": "success",
                            "gas_limit": gas_limit,
                            "gas_used": gas_used,
                            "output_hex": hex::encode(output),
                            "logs": logs,
                            "storage_writes": side_effects.storage_updates.writes.len(),
                            "storage_deletes": side_effects.storage_updates.deletes.len(),
                            "state_mutated": false
                        }),
                    ))
                }
                Err(e) => Ok(text_or_json(
                    json,
                    &format!(
                        "Contract simulation failed\nWASM: {}\nMethod: {}\nError: {}",
                        wasm.display(),
                        method,
                        e
                    ),
                    serde_json::json!({
                        "wasm": wasm.to_string_lossy(),
                        "method": method,
                        "args": args,
                        "code_size": wasm_data.len(),
                        "status": "error",
                        "gas_limit": gas_limit,
                        "error": e.to_string(),
                        "state_mutated": false
                    }),
                )),
            }
        }
        ContractCommands::EstimateGas { contract_id, method, args, .. } => {
            let body = serde_json::json!({
                "contract_id": contract_id,
                "method": method,
                "args": args.map(|a| vec![a]).unwrap_or_default(),
            })
            .to_string();
            let out = std::process::Command::new("curl")
                .args([
                    "-s",
                    "-X",
                    "POST",
                    "-H",
                    "Content-Type: application/json",
                    "-d",
                    &body,
                    "http://127.0.0.1:8080/api/v1/contracts/estimate-gas",
                ])
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    let text = String::from_utf8_lossy(&o.stdout).to_string();
                    let val: serde_json::Value =
                        serde_json::from_str(&text).unwrap_or(serde_json::json!({"raw": text}));
                    Ok(text_or_json(json, &format!("Gas estimate: {}", val), val))
                }
                Ok(_) | Err(_) => {
                    Err("Could not reach node — start with `baals node start` first".into())
                }
            }
        }
        ContractCommands::Abi { contract_id, .. } => {
            let url = format!("http://127.0.0.1:8080/api/v1/contracts/{}/abi", contract_id);
            let resp = ureq::get(&url).call();
            match resp {
                Ok(response) => {
                    let text = response.into_string().unwrap_or_default();
                    let val: serde_json::Value =
                        serde_json::from_str(&text).unwrap_or(serde_json::json!({"raw": text}));
                    Ok(text_or_json(json, &format!("ABI: {}", val), val))
                }
                Err(_) => Err("Could not reach node — start with `baals node start` first".into()),
            }
        }
        ContractCommands::VerifyWasm { wasm } => match std::fs::read(&wasm) {
            Ok(data) => {
                let is_valid = data.starts_with(b"\0asm");
                let size = data.len();

                if is_valid && size > 0 {
                    let mut hasher = Sha256::new();
                    Digest::update(&mut hasher, &data);
                    let hash = hex::encode(hasher.finalize());
                    Ok(text_or_json(
                        json,
                        &format!(
                            "WASM Verification: {}\nSize: {} bytes\nHash: {}\nStatus: Valid WASM",
                            wasm.display(),
                            size,
                            &hash[..16]
                        ),
                        serde_json::json!({
                            "wasm": wasm.to_string_lossy(),
                            "valid": true,
                            "size": size,
                            "hash": hash,
                            "magic_valid": is_valid
                        }),
                    ))
                } else {
                    Err("Invalid WASM: magic number check failed or empty file".to_string().into())
                }
            }
            Err(e) => Err(format!("Cannot read WASM file: {}", e).into()),
        },
    }
}
