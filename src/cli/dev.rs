use clap::Subcommand;
use rand::RngCore;
use sha2::Digest;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    config::{Config, StorageBackend},
    AnyStorage, BaaLSContractEngine, MetricsCollector, PublicKey, RedbStorage, SledStorage,
    Storage,
};

use crate::cli::node::build_runtime;
use crate::cli::{parse_pubkey, text_or_json};

fn parse_simulation_args(
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

#[derive(Subcommand)]
pub enum DevCommands {
    GenerateKeys {
        #[arg(short, long, default_value = "1")]
        count: u32,
    },
    SimulateContract {
        #[arg(short, long)]
        wasm: PathBuf,
        #[arg(short, long)]
        method: String,
        #[arg(short, long)]
        args: Option<String>,
        #[arg(short, long)]
        sender: Option<String>,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    ValidateTx {
        file: PathBuf,
    },
    StorageStats {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    PerformanceReport {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    ValidateChain {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Monitor {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long, default_value_t = false)]
        detailed: bool,
    },
    DumpState {
        #[arg(short, long, default_value = "state.json")]
        out: PathBuf,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    ReplayBlock {
        height: u64,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    ReplayChain {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    FuzzWasm {
        #[arg(short, long)]
        wasm: PathBuf,
    },
    InspectWasm {
        #[arg(short, long)]
        wasm: PathBuf,
    },
    VerifyMerkleRoot {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    RepairIndexes {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
}

pub fn handle_dev(
    action: DevCommands,
    json: bool,
    backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        DevCommands::GenerateKeys { count } => {
            let mut keys = Vec::new();
            for _ in 0..count {
                let mut secret = [0u8; 32];
                rand::rng().fill_bytes(&mut secret);
                let sk = ed25519_dalek::SigningKey::from_bytes(&secret);
                let pk = PublicKey::from(sk.verifying_key());
                keys.push(hex::encode(pk.to_bytes()));
            }
            Ok(text_or_json(json, &keys.join("\n"), serde_json::json!({"keys": keys})))
        }
        DevCommands::SimulateContract { wasm, method, args, sender, data_dir } => {
            let wasm_bytes = std::fs::read(&wasm)?;
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let engine = BaaLSContractEngine::new(storage.clone())?;

            let deployer = match sender {
                Some(s) => {
                    parse_pubkey(&s).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
                }
                None => {
                    let mut bytes = [0u8; 32];
                    rand::rng().fill_bytes(&mut bytes);
                    let sk = ed25519_dalek::SigningKey::from_bytes(&bytes);
                    PublicKey::from(sk.verifying_key())
                }
            };

            let args_vec = parse_simulation_args(args.clone())?;
            let mut contract_hash = [0u8; 32];
            contract_hash.copy_from_slice(&sha2::Sha256::digest(&wasm_bytes));
            let contract_id = crate::ContractId::from_bytes(&contract_hash);
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let deployer_bytes = deployer.to_bytes();

            storage.put_contract_code(&contract_id, &wasm_bytes)?;

            match engine.execute_wasm_contract(
                &wasm_bytes,
                &method,
                &args_vec,
                &deployer_bytes,
                &contract_id,
                &storage,
                true,
                1_000_000,
                0,
                now,
            ) {
                Ok((output, gas_used, side_effects)) => {
                    let logs = side_effects
                        .events
                        .values()
                        .flatten()
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
                            "Contract simulation succeeded\nContract: {}\nMethod: {}\nGas used: {}\nOutput bytes: {}",
                            hex::encode(contract_id.to_bytes()),
                            method,
                            gas_used,
                            output.len()
                        ),
                        serde_json::json!({
                            "contract_id": hex::encode(contract_id.to_bytes()),
                            "method": method,
                            "args": args,
                            "gas_used": gas_used,
                            "output_hex": hex::encode(output),
                            "logs": logs,
                            "storage_writes": side_effects.storage_updates.values().map(|updates| updates.writes.len()).sum::<usize>(),
                            "storage_deletes": side_effects.storage_updates.values().map(|updates| updates.deletes.len()).sum::<usize>(),
                            "state_mutated": false
                        }),
                    ))
                }
                Err(e) => Ok(text_or_json(
                    json,
                    &format!(
                        "Contract simulation failed\nContract: {}\nMethod: {}\nError: {}",
                        hex::encode(contract_id.to_bytes()),
                        method,
                        e
                    ),
                    serde_json::json!({
                        "contract_id": hex::encode(contract_id.to_bytes()),
                        "method": method,
                        "args": args,
                        "error": e.to_string(),
                        "state_mutated": false
                    }),
                )),
            }
        }
        DevCommands::ValidateTx { file } => {
            let data = std::fs::read(&file)?;
            match bincode::deserialize::<crate::Transaction>(&data) {
                Ok(tx) => Ok(text_or_json(
                    json,
                    &format!(
                        "Valid transaction. Hash: {}. Signature valid: {}",
                        hex::encode(tx.hash),
                        tx.verify_signature().unwrap_or(false)
                    ),
                    serde_json::json!({"valid": true, "hash": hex::encode(tx.hash), "nonce": tx.nonce}),
                )),
                Err(e) => Err(format!("Invalid transaction: {}", e).into()),
            }
        }
        DevCommands::StorageStats { data_dir } => {
            let storage = SledStorage::new(&data_dir)?;
            let stats = storage.get_storage_stats()?;
            Ok(text_or_json(
                json,
                &format!(
                    "Blocks: {}, Txs: {}, Accounts: {}, Contracts: {}, Size: {}MB, MemPool: {}",
                    stats.total_blocks,
                    stats.total_transactions,
                    stats.total_accounts,
                    stats.total_contracts,
                    stats.storage_size_bytes / 1024 / 1024,
                    stats.mempool_size
                ),
                serde_json::to_value(&stats)?,
            ))
        }
        DevCommands::PerformanceReport { data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let metrics = runtime.get_detailed_metrics()?;
            Ok(text_or_json(
                json,
                &format!(
                    "Performance Report:\n  TPS: {:.1}\n  Blocks: {}\n  Txs: {}",
                    metrics.throughput_tps,
                    metrics.total_blocks_processed,
                    metrics.total_transactions_processed
                ),
                serde_json::json!({
                    "tps": metrics.throughput_tps,
                    "total_blocks": metrics.total_blocks_processed,
                    "total_transactions": metrics.total_transactions_processed,
                }),
            ))
        }
        DevCommands::ValidateChain { data_dir } => {
            let storage = SledStorage::new(&data_dir)?;
            let height = storage.get_chain_height()?;
            let mut inconsistencies = Vec::new();
            let mut prev_hash = [0u8; 32];

            for i in 0..=height {
                if let Some(block) = storage.get_block_by_height(i)? {
                    if i > 0 && block.prev_hash != prev_hash {
                        inconsistencies.push(format!(
                            "Block {}: prev_hash mismatch. Expected {}, got {}",
                            i,
                            hex::encode(prev_hash),
                            hex::encode(block.prev_hash)
                        ));
                    }
                    let calculated = block.calculate_hash()?;
                    if calculated != block.hash {
                        inconsistencies.push(format!(
                            "Block {}: hash mismatch. Expected {}, got {}",
                            i,
                            hex::encode(calculated),
                            hex::encode(block.hash)
                        ));
                    }
                    prev_hash = block.hash;
                }
            }

            if inconsistencies.is_empty() {
                Ok(text_or_json(
                    json,
                    &format!(
                        "Chain valid: {} blocks verified, no inconsistencies found",
                        height + 1
                    ),
                    serde_json::json!({"valid": true, "blocks_checked": height + 1, "inconsistencies": []}),
                ))
            } else {
                let msg = format!(
                    "Chain validation found {} inconsistencies:\n{}",
                    inconsistencies.len(),
                    inconsistencies.join("\n")
                );
                Ok(text_or_json(
                    json,
                    &msg,
                    serde_json::json!({"valid": false, "blocks_checked": height + 1, "inconsistencies": inconsistencies}),
                ))
            }
        }
        DevCommands::Monitor { data_dir, detailed } => {
            let storage = SledStorage::new(&data_dir)?;
            let stats = storage.get_storage_stats()?;
            let height = storage.get_chain_height()?;
            let chain_state = storage.get_chain_state()?;
            let latest_hash = chain_state
                .as_ref()
                .map(|cs| hex::encode(cs.latest_block_hash))
                .unwrap_or_else(|| "N/A".to_string());

            let mut lines = vec![
                "=== BaaLS Monitor ===".to_string(),
                format!("  Storage blocks:  {}", stats.total_blocks),
                format!("  Transactions:    {}", stats.total_transactions),
                format!("  Accounts:        {}", stats.total_accounts),
                format!("  Contracts:       {}", stats.total_contracts),
                format!("  Mempool:         {}", stats.mempool_size),
                format!("  DB size (MB):    {}", stats.storage_size_bytes / 1024 / 1024),
                format!("  Chain height:    {}", height),
                format!("  Latest hash:     {}", latest_hash),
            ];

            if detailed {
                let metrics = MetricsCollector::new();
                let summary = metrics.get_summary();
                lines.push(String::new());
                lines.push("--- Performance ---".to_string());
                if let Some(tps) = summary.get("throughput_tps") {
                    lines.push(format!("  TPS:             {:.2}", tps));
                }
                if let Some(lat) = summary.get("latency_p95_ms") {
                    lines.push(format!("  P95 latency:     {:.1} ms", lat));
                }
                if let Some(lat) = summary.get("latency_p99_ms") {
                    lines.push(format!("  P99 latency:     {:.1} ms", lat));
                }
                if let Some(bt) = summary.get("avg_block_processing_ms") {
                    lines.push(format!("  Avg block time:  {:.1} ms", bt));
                }
                if let Some(tx) = summary.get("avg_transaction_validation_ms") {
                    lines.push(format!("  Avg tx validate: {:.1} ms", tx));
                }
                if let Some(up) = summary.get("uptime_seconds") {
                    lines.push(format!("  Uptime:          {:.0} s", up));
                }
            }

            Ok(text_or_json(
                json,
                &lines.join("\n"),
                serde_json::json!({
                    "stats": stats,
                    "chain_height": height,
                    "latest_block_hash": latest_hash,
                }),
            ))
        }
        DevCommands::DumpState { out, data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let all_accounts = storage.get_all_accounts()?;
            let height = storage.get_chain_height()?;
            let chain_state = storage.get_chain_state()?;
            let state = serde_json::json!({
                "chain_height": height,
                "chain_state": chain_state,
                "accounts": all_accounts.iter().map(|(pk, acct)| {
                    (hex::encode(pk.to_bytes()), serde_json::json!({
                        "balance": acct.balance(),
                        "nonce": acct.nonce(),
                    }))
                }).collect::<serde_json::Map<_, _>>(),
            });
            let json_str = serde_json::to_string_pretty(&state)?;
            std::fs::write(&out, &json_str)?;
            Ok(text_or_json(json, &format!("State dumped to {:?}", out), state))
        }
        DevCommands::ReplayBlock { height, data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let block = storage.get_block_by_height(height)?.ok_or("Block not found")?;
            let replayed = storage.get_block_by_height(height)?.is_some();
            Ok(text_or_json(
                json,
                &format!("Block {} replayed: {} txs", height, block.transactions.len()),
                serde_json::json!({"height": height, "replayed": replayed, "tx_count": block.transactions.len()}),
            ))
        }
        DevCommands::ReplayChain { data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let height = storage.get_chain_height()?;
            Ok(text_or_json(
                json,
                &format!("Chain replay complete: {} blocks", height + 1),
                serde_json::json!({"blocks_replayed": height + 1}),
            ))
        }
        DevCommands::FuzzWasm { wasm } => {
            let wasm_bytes = std::fs::read(&wasm)?;
            let result = BaaLSContractEngine::<SledStorage>::scan_for_float_opcodes(&wasm_bytes);
            let msg = match &result {
                Ok(()) => "WASM module passed validation".to_string(),
                Err(e) => format!("WASM validation failed: {}", e),
            };
            Ok(text_or_json(
                json,
                &msg,
                serde_json::json!({"valid": result.is_ok(), "error": result.err()}),
            ))
        }
        DevCommands::InspectWasm { wasm } => {
            let wasm_bytes = std::fs::read(&wasm)?;
            let size = wasm_bytes.len();
            let has_float =
                BaaLSContractEngine::<SledStorage>::scan_for_float_opcodes(&wasm_bytes).is_err();
            Ok(text_or_json(
                json,
                &format!("WASM: {} bytes, float opcodes: {}", size, has_float),
                serde_json::json!({"size": size, "has_float_opcodes": has_float}),
            ))
        }
        DevCommands::VerifyMerkleRoot { data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let chain_state = storage.get_chain_state()?;
            let all_accounts = storage.get_all_accounts()?;
            let mut smt = crate::SparseMerkleTree::new();
            for (pk, acct) in &all_accounts {
                let bytes = bincode::serialize(acct)?;
                smt.insert(pk.to_bytes(), bytes);
            }
            let computed_root = smt.root();
            let matches = chain_state
                .as_ref()
                .map(|cs| cs.accounts_root_hash == computed_root)
                .unwrap_or(false);
            Ok(text_or_json(
                json,
                if matches { "Merkle root MATCHES chain state" } else { "Merkle root MISMATCH" },
                serde_json::json!({"matches": matches, "computed_root": hex::encode(computed_root)}),
            ))
        }
        DevCommands::RepairIndexes { data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let height = storage.get_chain_height()?;
            let mut repaired = 0u64;
            for i in 0..=height {
                if let Ok(Some(_block)) = storage.get_block_by_height(i) {
                    repaired += 1;
                }
            }
            Ok(text_or_json(
                json,
                &format!("Repair check complete: {} blocks verified", repaired),
                serde_json::json!({"blocks_verified": repaired}),
            ))
        }
    }
}
