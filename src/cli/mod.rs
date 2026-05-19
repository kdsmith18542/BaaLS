use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub mod admin;
pub mod contract;
pub mod db;
pub mod dev;
pub mod node;
pub mod p2p;
pub mod proof;
pub mod query;
pub mod tx;
pub mod wallet;

#[derive(Parser)]
#[command(name = "baalsd")]
#[command(about = "BaaLS - Blockchain as a Local Service")]
#[command(version)]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,

    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[arg(long, global = true, default_value = "sled")]
    pub storage_backend: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Node {
        #[command(subcommand)]
        action: node::NodeCommands,
    },
    Wallet {
        #[command(subcommand)]
        action: wallet::WalletCommands,
    },
    Tx {
        #[command(subcommand)]
        action: tx::TxCommands,
    },
    Query {
        #[command(subcommand)]
        action: query::QueryCommands,
    },
    Dev {
        #[command(subcommand)]
        action: dev::DevCommands,
    },
    Db {
        #[command(subcommand)]
        command: db::DbCommands,
    },
    Key {
        #[command(subcommand)]
        action: KeyCommands,
    },
    Proof {
        #[command(subcommand)]
        action: proof::ProofCommands,
    },
    P2p {
        #[command(subcommand)]
        action: p2p::P2pCommands,
    },
    Contract {
        #[command(subcommand)]
        action: contract::ContractCommands,
    },
    Admin {
        #[command(subcommand)]
        action: admin::AdminCommands,
    },
    Api {
        #[command(subcommand)]
        action: ApiCommands,
    },
    Doctor,
}

#[derive(Subcommand)]
pub enum KeyCommands {
    Generate,
    Inspect { key_hex: String },
    Sign { key_hex: String, message: String },
    Verify { key_hex: String, message: String, signature: String },
}

#[derive(Subcommand)]
pub enum ApiCommands {
    Token {
        #[arg(short, long, default_value = "http://localhost:8080")]
        endpoint: String,
        #[arg(long)]
        private_key: String,
        #[arg(long, default_value_t = 900)]
        ttl_seconds: u64,
    },
    Health {
        #[arg(short, long, default_value = "http://localhost:8080")]
        endpoint: String,
    },
    Submit {
        #[arg(short, long)]
        file: PathBuf,
        #[arg(short, long, default_value = "http://localhost:8080")]
        endpoint: String,
        #[arg(short, long)]
        token: String,
    },
    Deploy {
        #[arg(short, long)]
        wasm: PathBuf,
        #[arg(short, long)]
        deployer: String,
        #[arg(long, default_value_t = 1_000_000)]
        gas_limit: u64,
        #[arg(long)]
        init_hex: Option<String>,
        #[arg(short, long, default_value = "http://localhost:8080")]
        endpoint: String,
        #[arg(short, long)]
        token: String,
    },
    Call {
        #[arg(short, long)]
        contract_id: String,
        #[arg(short, long)]
        method: String,
        #[arg(short, long)]
        args: Option<String>,
        #[arg(short, long, default_value = "http://localhost:8080")]
        endpoint: String,
        #[arg(short, long)]
        token: Option<String>,
    },
}

pub fn json_err(msg: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({"error": true, "message": msg}))
        .unwrap_or_default()
}

pub fn text_or_json(json: bool, text: &str, json_val: serde_json::Value) -> String {
    if json {
        serde_json::to_string_pretty(&json_val).unwrap_or_default()
    } else {
        text.to_string()
    }
}

pub fn parse_pubkey(hex_str: &str) -> Result<crate::PublicKey, String> {
    let bytes = hex::decode(hex_str).map_err(|_| "Invalid hex".to_string())?;
    if bytes.len() != 32 {
        return Err("Public key must be 32 bytes hex".to_string());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    crate::PublicKey::from_bytes(&arr).map_err(|e| format!("Invalid public key: {:?}", e))
}

pub fn block_to_json(block: &crate::Block) -> serde_json::Value {
    use crate::{Address, TransactionPayload};
    let txs: Vec<serde_json::Value> = block
        .transactions
        .iter()
        .map(|tx| {
            let from = hex::encode(tx.sender.to_bytes());
            let to = match &tx.recipient {
                Address::Wallet(pk) => hex::encode(pk.to_bytes()),
                Address::Contract(cid) => hex::encode(cid.to_bytes()),
            };
            let value: u64 = match &tx.payload {
                TransactionPayload::Transfer { amount } => *amount,
                TransactionPayload::ContractCall { value: Some(v), .. } => *v,
                _ => 0,
            };
            serde_json::json!({
                "hash": hex::encode(tx.hash),
                "from": from,
                "to": to,
                "value": value.to_string(),
                "nonce": tx.nonce,
                "gas": tx.gas_limit,
                "gasUsed": 0,
            })
        })
        .collect();

    serde_json::json!({
        "hash": hex::encode(block.hash),
        "parentHash": hex::encode(block.prev_hash),
        "number": block.index,
        "height": block.index,
        "timestamp": block.timestamp,
        "transactions": txs,
    })
}
