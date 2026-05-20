use clap::Subcommand;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::{
    config::{Config, StorageBackend},
    Account, Address, Transaction, TransactionPayload, TransactionSignature,
};

use crate::cli::node::build_runtime;
use crate::cli::{parse_pubkey, text_or_json};

#[derive(Subcommand)]
pub enum TxCommands {
    Transfer {
        #[arg(short, long)]
        sender: String,
        #[arg(short, long)]
        recipient: String,
        #[arg(short, long)]
        amount: u64,
        #[arg(short, long)]
        memo: Option<String>,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    DeployContract {
        #[arg(short, long)]
        sender: String,
        #[arg(short, long)]
        wasm: PathBuf,
        #[arg(long)]
        init_args: Option<String>,
        #[arg(short, long, default_value = "1000000")]
        gas_limit: u64,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    CallContract {
        #[arg(short, long)]
        sender: String,
        #[arg(short, long)]
        contract_id: String,
        #[arg(short, long)]
        method: String,
        #[arg(short, long)]
        args: Option<String>,
        #[arg(short, long, default_value = "0")]
        value: u64,
        #[arg(short, long, default_value = "1000000")]
        gas_limit: u64,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Data {
        #[arg(short, long)]
        sender: String,
        #[arg(short, long)]
        data: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Inspect {
        file: PathBuf,
    },
    EstimateFee {
        #[arg(short, long)]
        file: PathBuf,
        #[arg(short, long, default_value = "1")]
        gas_price: u64,
    },
    Sign {
        #[arg(short, long)]
        file: PathBuf,
        #[arg(short, long)]
        wallet: String,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    Submit {
        #[arg(short, long)]
        file: PathBuf,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Validate {
        #[arg(short, long)]
        file: PathBuf,
    },
    Decode {
        #[arg(short, long)]
        file: PathBuf,
    },
}

fn prompt_password(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut stdout = io::stdout();
    write!(stdout, "{}", prompt)?;
    stdout.flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

pub fn handle_tx(
    action: TxCommands,
    json: bool,
    backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let keystore = crate::Keystore::new(None)?;

    match action {
        TxCommands::Transfer { sender, recipient, amount, memo, data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let sender_pk = parse_pubkey(&sender)?;
            let recipient_pk = parse_pubkey(&recipient)?;

            let sender_account = runtime
                .get_account(&sender_pk)?
                .unwrap_or(Account::Wallet { balance: 0, nonce: 0 });
            let nonce = sender_account.nonce() + 1;
            if let Account::Wallet { balance, .. } = sender_account {
                if balance < amount {
                    return Err(format!("Insufficient balance: {} < {}", balance, amount).into());
                }
            }

            let password = prompt_password("Wallet password: ")?;
            let signing_key = keystore.load_key(&sender_pk, &password)?;

            let mut tx = Transaction {
                hash: [0u8; 32],
                sender: sender_pk,
                recipient: Address::Wallet(recipient_pk),
                payload: TransactionPayload::Transfer { amount },
                nonce,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                signature: TransactionSignature::from_bytes(&[0u8; 64])?,
                gas_limit: 100_000,
                gas_price: 1,
                priority: 0,
                metadata: memo.map(|m| {
                    let mut map = std::collections::BTreeMap::new();
                    map.insert("memo".to_string(), m);
                    map
                }),
                chain_id: 1,
            };
            tx.hash = tx.calculate_hash()?;
            tx.sign(&signing_key)?;
            runtime.submit_transaction(tx.clone())?;

            let tokio_rt = tokio::runtime::Runtime::new()?;
            let block = tokio_rt.block_on(runtime.produce_block())?;
            Ok(text_or_json(
                json,
                &format!(
                    "Transfer: {} -> {} ({})\nTx: {}\nBlock: {}",
                    &sender[..8],
                    &recipient[..8],
                    amount,
                    hex::encode(tx.hash),
                    block.index
                ),
                serde_json::json!({"tx_hash": hex::encode(tx.hash), "block": block.index}),
            ))
        }
        TxCommands::DeployContract { sender, wasm, init_args, gas_limit, data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let sender_pk = parse_pubkey(&sender)?;
            let wasm_bytes = std::fs::read(&wasm)?;
            let _account = runtime
                .get_account(&sender_pk)?
                .unwrap_or(Account::Wallet { balance: 0, nonce: 0 });

            let password = prompt_password("Wallet password: ")?;
            let _signing_key = keystore.load_key(&sender_pk, &password)?;

            let init_payload = init_args.map(std::fs::read).transpose()?;
            let init_payload_ref = init_payload.as_deref();

            let cid =
                runtime.deploy_contract(&sender_pk, &wasm_bytes, init_payload_ref, gas_limit)?;
            Ok(text_or_json(
                json,
                &format!("Contract deployed: {}", hex::encode(cid.to_bytes())),
                serde_json::json!({"contract_id": hex::encode(cid.to_bytes())}),
            ))
        }
        TxCommands::CallContract {
            sender,
            contract_id,
            method,
            args,
            value,
            gas_limit,
            data_dir,
        } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let sender_pk = parse_pubkey(&sender)?;
            let cid_bytes = hex::decode(&contract_id)?;
            if cid_bytes.len() != 32 {
                return Err("Contract ID must be 32 bytes hex".into());
            }
            let mut cid_arr = [0u8; 32];
            cid_arr.copy_from_slice(&cid_bytes);
            let cid = crate::ContractId::from_bytes(&cid_arr);

            let arg_bytes = args.map(|a| vec![a.into_bytes()]).unwrap_or_default();
            let call_value = if value > 0 { Some(value) } else { None };
            let result = runtime
                .call_contract(&sender_pk, &cid, &method, &arg_bytes, call_value, gas_limit)?;

            Ok(text_or_json(
                json,
                &format!(
                    "Result ({} bytes): {}",
                    result.len(),
                    hex::encode(&result[..result.len().min(64)])
                ),
                serde_json::json!({"result_hex": hex::encode(&result), "result_len": result.len()}),
            ))
        }
        TxCommands::Data { sender, data, data_dir } => {
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            let sender_pk = parse_pubkey(&sender)?;
            let account = runtime
                .get_account(&sender_pk)?
                .unwrap_or(Account::Wallet { balance: 0, nonce: 0 });
            let nonce = account.nonce() + 1;

            let password = prompt_password("Wallet password: ")?;
            let signing_key = keystore.load_key(&sender_pk, &password)?;

            let data_bytes = hex::decode(&data).unwrap_or_else(|_| data.as_bytes().to_vec());
            let mut tx = Transaction {
                hash: [0u8; 32],
                sender: sender_pk,
                recipient: Address::Wallet(sender_pk),
                payload: TransactionPayload::Data { data: data_bytes },
                nonce,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                signature: TransactionSignature::from_bytes(&[0u8; 64])?,
                gas_limit: 100_000,
                gas_price: 1,
                priority: 0,
                metadata: None,
                chain_id: 1,
            };
            tx.hash = tx.calculate_hash()?;
            tx.sign(&signing_key)?;
            runtime.submit_transaction(tx.clone())?;
            Ok(text_or_json(
                json,
                &format!("Data tx submitted: {}", hex::encode(tx.hash)),
                serde_json::json!({"tx_hash": hex::encode(tx.hash)}),
            ))
        }
        TxCommands::Inspect { file } => {
            let data = std::fs::read(&file)?;
            match bincode::deserialize::<Transaction>(&data) {
                Ok(tx) => Ok(text_or_json(
                    json,
                    &format!(
                        "Tx: hash={}, sender={}, nonce={}, amount={:?}",
                        hex::encode(tx.hash),
                        hex::encode(&tx.sender.to_bytes()[..8]),
                        tx.nonce,
                        tx.payload
                    ),
                    serde_json::to_value(serde_json::json!({
                        "hash": hex::encode(tx.hash),
                        "sender": hex::encode(tx.sender.to_bytes()),
                        "nonce": tx.nonce,
                    }))?,
                )),
                Err(e) => Err(format!("Failed to parse transaction: {}", e).into()),
            }
        }
        TxCommands::EstimateFee { file, gas_price } => {
            let data = std::fs::read_to_string(&file)?;
            let tx: Transaction = serde_json::from_str(&data)?;
            let base_gas: u64 = 21_000;
            let payload_gas: u64 = match &tx.payload {
                TransactionPayload::Transfer { .. } => 0,
                TransactionPayload::ContractDeploy { wasm_bytes, .. } => {
                    100_000 + (wasm_bytes.len() as u64 / 100) * 100
                }
                TransactionPayload::ContractCall { args, .. } => {
                    50_000 + args.iter().map(|a| a.len() as u64).sum::<u64>() * 10
                }
                TransactionPayload::Data { data } => 1_000 + data.len() as u64,
                TransactionPayload::ValidatorSetChange { added, removed, .. } => {
                    5_000 + (added.len() + removed.len()) as u64 * 100
                }
            };
            let total_gas = base_gas + payload_gas;
            let total_fee = gas_price.checked_mul(total_gas).ok_or("Fee overflow")?;
            Ok(text_or_json(
                json,
                &format!(
                    "Estimated gas: {} units\nGas price: {}\nEstimated fee: {}\n",
                    total_gas, gas_price, total_fee
                ),
                serde_json::json!({
                    "gas_used": total_gas,
                    "gas_price": gas_price,
                    "estimated_fee": total_fee,
                }),
            ))
        }
        TxCommands::Sign { file, wallet, out } => {
            let data = std::fs::read_to_string(&file)?;
            let mut tx: Transaction = serde_json::from_str(&data)?;
            let pk = parse_pubkey(&wallet)?;
            let keystore = crate::Keystore::new(None)?;
            let password = prompt_password("Wallet password: ")?;
            let sk = keystore.load_key(&pk, &password)?;
            tx.sender = pk;
            tx.sign(&sk)?;
            let out_path = out.unwrap_or_else(|| PathBuf::from("signed_tx.json"));
            let tx_json = serde_json::to_string_pretty(&tx)?;
            std::fs::write(&out_path, &tx_json)?;
            Ok(text_or_json(
                json,
                &format!("Signed transaction saved to {:?}", out_path),
                serde_json::json!({"signed": true, "output": out_path.to_string_lossy().to_string()}),
            ))
        }
        TxCommands::Submit { file, data_dir } => {
            let data = std::fs::read_to_string(&file)?;
            let tx: Transaction = serde_json::from_str(&data)?;
            let mut cfg = Config::default();
            cfg.storage.backend = match backend {
                "redb" => StorageBackend::Redb,
                _ => StorageBackend::Sled,
            };
            let (runtime, _, _) = build_runtime(&data_dir, &cfg, &[], "0.0.0.0:9070", false)?;
            runtime.submit_transaction(tx.clone())?;
            let tokio_rt = tokio::runtime::Runtime::new()?;
            let block = tokio_rt.block_on(runtime.produce_block())?;
            Ok(text_or_json(
                json,
                &format!("Tx submitted: {}\nBlock: {}", hex::encode(tx.hash), block.index),
                serde_json::json!({"tx_hash": hex::encode(tx.hash), "block": block.index}),
            ))
        }
        TxCommands::Validate { file } => {
            let data = std::fs::read(&file)?;
            match bincode::deserialize::<Transaction>(&data) {
                Ok(tx) => {
                    let hash_ok = tx.calculate_hash().map(|h| h == tx.hash).unwrap_or(false);
                    let sig_ok = tx.verify_signature().unwrap_or(false);
                    let valid = hash_ok && sig_ok;
                    Ok(text_or_json(
                        json,
                        if valid { "Transaction is VALID" } else { "Transaction is INVALID" },
                        serde_json::json!({"valid": valid, "hash_match": hash_ok, "signature_valid": sig_ok}),
                    ))
                }
                Err(e) => Err(format!("Failed to parse transaction: {}", e).into()),
            }
        }
        TxCommands::Decode { file } => {
            let data = std::fs::read(&file)?;
            let tx: Transaction = if file.extension().map(|e| e == "json").unwrap_or(false) {
                serde_json::from_slice(&data)?
            } else {
                bincode::deserialize(&data)?
            };
            let json_val = serde_json::json!({
                "hash": hex::encode(tx.hash),
                "sender": hex::encode(tx.sender.to_bytes()),
                "nonce": tx.nonce,
                "timestamp": tx.timestamp,
                "recipient": match &tx.recipient {
                    Address::Wallet(pk) => hex::encode(pk.to_bytes()),
                    Address::Contract(cid) => hex::encode(cid.to_bytes()),
                },
                "payload": format!("{:?}", tx.payload),
                "gas_limit": tx.gas_limit,
                "gas_price": tx.gas_price,
                "priority": tx.priority,
                "signature": hex::encode(tx.signature.to_bytes()),
            });
            Ok(text_or_json(json, &serde_json::to_string_pretty(&json_val)?, json_val))
        }
    }
}
