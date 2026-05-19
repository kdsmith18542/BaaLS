use clap::Subcommand;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

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
        ContractCommands::Simulate { wasm, method, args } => {
            let wasm_data = std::fs::read(&wasm)?;

            if !wasm_data.starts_with(b"\0asm") {
                return Err("Invalid WASM magic number".into());
            }

            let code_size = wasm_data.len();
            let args_str = args.as_deref().unwrap_or("{}");

            Ok(text_or_json(
                json,
                &format!(
                    "Contract simulation for: {}\nMethod: {}\nArgs: {}\nCode size: {} bytes\n(Note: Actual execution requires runtime context)",
                    wasm.display(), method, args_str, code_size
                ),
                serde_json::json!({
                    "wasm": wasm.to_string_lossy(),
                    "method": method,
                    "args": args,
                    "code_size": code_size,
                    "valid": true,
                    "status": "validation_only"
                }),
            ))
        }
        ContractCommands::EstimateGas { contract_id, method, args, .. } => {
            let msg = format!(
                "contract estimate-gas is not implemented for live runtime execution yet \
(contract_id={}, method={}, args_present={}).",
                contract_id,
                method,
                args.is_some()
            );
            let _ = text_or_json(
                json,
                &msg,
                serde_json::json!({
                    "error": "not_implemented",
                    "command": "contract estimate-gas",
                    "contract_id": contract_id,
                    "method": method,
                    "args_present": args.is_some(),
                    "message": msg,
                }),
            );
            Err(msg.into())
        }
        ContractCommands::Abi { contract_id, .. } => {
            let msg = format!(
                "contract abi is not implemented for deployed artifact introspection yet (contract_id={}).",
                contract_id
            );
            let _ = text_or_json(
                json,
                &msg,
                serde_json::json!({
                    "error": "not_implemented",
                    "command": "contract abi",
                    "contract_id": contract_id,
                    "message": msg,
                }),
            );
            Err(msg.into())
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
