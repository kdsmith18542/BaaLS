use clap::Subcommand;
use std::path::PathBuf;

use crate::{
    config::{Config, StorageBackend},
    contract_account_public_key, Account, AnyStorage, ContractId, RedbStorage, SledStorage,
    Storage,
};

use crate::cli::node::build_runtime;
use crate::cli::{parse_pubkey, text_or_json};

#[derive(Subcommand)]
pub enum QueryCommands {
    Head {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Block {
        identifier: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Tx {
        hash: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Account {
        address: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Supply {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    ContractState {
        #[arg(short, long)]
        contract_id: String,
        #[arg(short, long)]
        key: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    ContractCall {
        #[arg(short, long)]
        contract_id: String,
        #[arg(short, long)]
        method: String,
        #[arg(short, long)]
        args: Option<String>,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Blocks {
        #[arg(short, long, default_value = "0")]
        from: u64,
        #[arg(short, long, default_value = "100")]
        to: u64,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    TxsByAccount {
        address: String,
        #[arg(short, long, default_value = "50")]
        limit: usize,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    TxsByContract {
        contract_id: String,
        #[arg(short, long, default_value = "50")]
        limit: usize,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Mempool {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
}

pub fn handle_query(
    action: QueryCommands,
    json: bool,
    backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        QueryCommands::Head { data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let chain = runtime.get_chain_state()?;
            let block = runtime.get_block(&chain.latest_block_hash)?.ok_or("No block found")?;
            Ok(text_or_json(
                json,
                &format!(
                    "Head: height={}, hash={}, txs={}",
                    chain.latest_block_index,
                    hex::encode(chain.latest_block_hash),
                    block.transactions.len()
                ),
                serde_json::json!({"height": chain.latest_block_index, "hash": hex::encode(chain.latest_block_hash)}),
            ))
        }
        QueryCommands::Block { identifier, data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let block = if let Ok(h) = hex::decode(&identifier) {
                if h.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&h);
                    runtime.get_block(&arr)?
                } else {
                    None
                }
            } else if let Ok(height) = identifier.parse::<u64>() {
                runtime.get_block_by_height(height)?
            } else {
                None
            };

            match block {
                Some(b) => Ok(text_or_json(
                    json,
                    &format!(
                        "Block #{}: hash={}, prev={}, txs={}, ts={}",
                        b.index,
                        hex::encode(b.hash),
                        hex::encode(&b.prev_hash[..8]),
                        b.transactions.len(),
                        b.timestamp
                    ),
                    serde_json::json!({
                        "index": b.index, "hash": hex::encode(b.hash), "timestamp": b.timestamp,
                        "transactions": b.transactions.len()
                    }),
                )),
                None => Err("Block not found".into()),
            }
        }
        QueryCommands::Tx { hash, data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let h = hex::decode(&hash)?;
            if h.len() != 32 {
                return Err("Hash must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&h);
            let mut tx = runtime.get_transaction(&arr)?;
            if tx.is_none() {
                tx = runtime.get_pending_transactions()?.into_iter().find(|t| t.hash == arr);
            }
            match tx {
                Some(tx) => {
                    let status = runtime
                        .get_transaction_status(&arr)?
                        .unwrap_or(crate::types::TransactionStatus::Pending);
                    let finality = runtime.get_transaction_finality(&arr)?;
                    let finality_text = finality
                        .as_ref()
                        .map(|f| format!("confirmations={}/{}", f.confirmations, f.required))
                        .unwrap_or_else(|| "confirmations=unknown".to_string());
                    Ok(text_or_json(
                        json,
                        &format!(
                            "Tx: sender={}, nonce={}, status={:?}, {}, payload={:?}",
                            hex::encode(&tx.sender.to_bytes()[..8]),
                            tx.nonce,
                            status,
                            finality_text,
                            tx.payload
                        ),
                        serde_json::json!({
                            "hash": hex::encode(tx.hash),
                            "sender": hex::encode(tx.sender.to_bytes()),
                            "nonce": tx.nonce,
                            "status": status,
                            "finality": finality,
                            "transaction": tx
                        }),
                    ))
                }
                None => Err("Transaction not found".into()),
            }
        }
        QueryCommands::Account { address, data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let pk = parse_pubkey(&address)?;
            match runtime.get_account(&pk)? {
                Some(Account::Wallet { balance, nonce }) => Ok(text_or_json(
                    json,
                    &format!("Wallet: balance={}, nonce={}", balance, nonce),
                    serde_json::json!({"type": "wallet", "balance": balance, "nonce": nonce}),
                )),
                Some(Account::Contract { code_hash, storage_root_hash, nonce, .. }) => {
                    Ok(text_or_json(
                        json,
                        &format!(
                            "Contract: code={}, storage={}, nonce={}",
                            hex::encode(&code_hash[..8]),
                            hex::encode(&storage_root_hash[..8]),
                            nonce
                        ),
                        serde_json::json!({"type": "contract", "code_hash": hex::encode(code_hash), "nonce": nonce}),
                    ))
                }
                None => Err("Account not found".into()),
            }
        }
        QueryCommands::Supply { data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let chain = runtime.get_chain_state()?;
            Ok(text_or_json(
                json,
                &format!(
                    "Supply: total_supply={}, height={}",
                    chain.total_supply, chain.latest_block_index
                ),
                serde_json::json!({
                    "total_supply": chain.total_supply,
                    "height": chain.latest_block_index,
                    "latest_block_hash": hex::encode(chain.latest_block_hash)
                }),
            ))
        }
        QueryCommands::ContractState { contract_id, key, data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let cid_bytes = hex::decode(&contract_id)?;
            if cid_bytes.len() != 32 {
                return Err("CID must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&cid_bytes);
            let cid = ContractId::from_bytes(&arr);
            let key_bytes = hex::decode(&key)?;
            if let Some(val) = runtime.contract_storage_read(&cid, &key_bytes)? {
                Ok(text_or_json(
                    json,
                    &hex::encode(&val),
                    serde_json::json!({"value_hex": hex::encode(&val), "exists": true}),
                ))
            } else {
                Ok(text_or_json(
                    json,
                    "Key not found in contract storage",
                    serde_json::json!({"exists": false}),
                ))
            }
        }
        QueryCommands::ContractCall { contract_id, method, args, data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let cid_bytes = hex::decode(&contract_id)?;
            if cid_bytes.len() != 32 {
                return Err("CID must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&cid_bytes);
            let cid = ContractId::from_bytes(&arr);
            let arg_bytes = args.map(|a| a.into_bytes()).unwrap_or_default();
            let result = runtime.query_contract(&cid, &method, &arg_bytes)?;
            Ok(text_or_json(
                json,
                &format!("Query result ({} bytes)", result.len()),
                serde_json::json!({"result_hex": hex::encode(&result)}),
            ))
        }
        QueryCommands::Blocks { from, to, data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let mut blocks = Vec::new();
            for h in from..=to.min(from + 100) {
                if let Some(block) = storage.get_block_by_height(h)? {
                    blocks.push(serde_json::json!({
                        "height": block.index,
                        "hash": hex::encode(block.hash),
                        "tx_count": block.transactions.len(),
                        "timestamp": block.timestamp,
                    }));
                }
            }
            Ok(text_or_json(
                json,
                &blocks
                    .iter()
                    .map(|b| serde_json::to_string(b).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\n"),
                serde_json::json!({"blocks": blocks, "count": blocks.len()}),
            ))
        }
        QueryCommands::TxsByAccount { address, limit, data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let pk = parse_pubkey(&address)?;
            let txs = storage.get_transactions_by_address(&pk, limit)?;
            let tx_list: Vec<serde_json::Value> = txs
                .iter()
                .map(|tx| {
                    serde_json::json!({
                        "hash": hex::encode(tx.hash),
                        "nonce": tx.nonce,
                        "timestamp": tx.timestamp,
                    })
                })
                .collect();
            Ok(text_or_json(
                json,
                &format!("Found {} transactions", tx_list.len()),
                serde_json::json!({"transactions": tx_list, "count": tx_list.len()}),
            ))
        }
        QueryCommands::TxsByContract { contract_id, limit, data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let cid_bytes = hex::decode(&contract_id)?;
            if cid_bytes.len() != 32 {
                return Err("Contract ID must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&cid_bytes);
            let contract_id = ContractId::from_bytes(&arr);
            let pk = contract_account_public_key(&contract_id);
            let txs = storage.get_transactions_by_address(&pk, limit)?;
            let tx_list: Vec<serde_json::Value> = txs
                .iter()
                .map(|tx| {
                    serde_json::json!({
                        "hash": hex::encode(tx.hash),
                        "nonce": tx.nonce,
                        "timestamp": tx.timestamp,
                    })
                })
                .collect();
            Ok(text_or_json(
                json,
                &format!("Found {} transactions for contract", tx_list.len()),
                serde_json::json!({"transactions": tx_list, "count": tx_list.len()}),
            ))
        }
        QueryCommands::Mempool { data_dir } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let pending = storage.get_pending_transactions()?;
            let tx_list: Vec<serde_json::Value> = pending
                .iter()
                .map(|tx| {
                    serde_json::json!({
                        "hash": hex::encode(tx.hash),
                        "sender": hex::encode(tx.sender.to_bytes()),
                        "nonce": tx.nonce,
                        "timestamp": tx.timestamp,
                    })
                })
                .collect();
            Ok(text_or_json(
                json,
                &format!("Mempool: {} pending transactions", tx_list.len()),
                serde_json::json!({"pending": tx_list, "count": tx_list.len()}),
            ))
        }
    }
}
