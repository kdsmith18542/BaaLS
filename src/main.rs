use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use baals::consensus::PoAConsensus;
use baals::contracts::{BaaLSContractEngine, ContractEngine};
use baals::runtime::Runtime;
use baals::storage::SledStorage;
use baals::sync::NoopSync;
use baals::types::{format_hex, Address, ContractId, PublicKey, Transaction, TransactionPayload};

#[derive(Parser)]
#[command(name = "baals")]
#[command(about = "BaaLS Blockchain CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Wallet operations
    Wallet {
        #[command(subcommand)]
        action: WalletCommands,
    },
    /// Transaction operations
    Transaction {
        #[command(subcommand)]
        action: TransactionCommands,
    },
    /// Query operations
    Query {
        #[command(subcommand)]
        action: QueryCommands,
    },
    /// Development operations
    Dev {
        #[command(subcommand)]
        action: DevCommands,
    },
}

#[derive(Subcommand)]
enum WalletCommands {
    /// Generate a new wallet
    Generate,
    /// Show wallet info
    Info {
        /// Private key file path
        #[arg(short, long)]
        key_file: PathBuf,
    },
}

#[derive(Subcommand)]
enum TransactionCommands {
    /// Send a transfer transaction
    Transfer {
        /// Private key file path
        #[arg(short, long)]
        key_file: PathBuf,
        /// Recipient address (hex)
        #[arg(short, long)]
        to: String,
        /// Amount to transfer
        #[arg(short, long)]
        amount: u64,
    },
    /// Deploy a smart contract
    Deploy {
        /// Private key file path
        #[arg(short, long)]
        key_file: PathBuf,
        /// Contract WASM file path
        #[arg(short, long)]
        contract: PathBuf,
    },
    /// Call a smart contract
    Call {
        /// Private key file path
        #[arg(short, long)]
        key_file: PathBuf,
        /// Contract ID (hex)
        #[arg(short, long)]
        contract_id: String,
        /// Method name
        #[arg(short, long)]
        method: String,
        /// Arguments (JSON)
        #[arg(short, long)]
        args: String,
    },
    /// Send data transaction
    Data {
        /// Private key file path
        #[arg(short, long)]
        key_file: PathBuf,
        /// Data payload (hex)
        #[arg(short, long)]
        data: String,
    },
}

#[derive(Subcommand)]
enum QueryCommands {
    /// Get block by height
    Block {
        /// Block height
        #[arg(short, long)]
        height: u64,
    },
    /// Get account info
    Account {
        /// Account address (hex)
        #[arg(short, long)]
        address: String,
    },
    /// Query contract storage
    Storage {
        /// Contract ID (hex)
        #[arg(short, long)]
        contract_id: String,
        /// Storage key (hex)
        #[arg(short, long)]
        key: String,
    },
    /// Query contract
    Contract {
        /// Contract ID (hex)
        #[arg(short, long)]
        contract_id: String,
        /// Query payload (hex)
        #[arg(short, long)]
        payload: String,
    },
}

#[derive(Subcommand)]
enum DevCommands {
    /// Start the node
    Start {
        /// Data directory
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    /// Generate a test block
    GenerateBlock,
    /// Show chain state
    ChainState,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Wallet { action } => match action {
            WalletCommands::Generate => {
                let signing_key =
                    Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key()?;
                let public_key = PublicKey::from(signing_key.verifying_key());
                println!("Generated new wallet:");
                println!("Public Key: {}", format_hex(&public_key.to_bytes()));
                println!("Private Key: {}", format_hex(&signing_key.to_bytes()));
            }
            WalletCommands::Info { key_file } => {
                let key_bytes = std::fs::read(key_file)?;
                let key_array: [u8; 32] = key_bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| "Invalid key length")?;
                let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_array);
                let public_key = PublicKey::from(signing_key.verifying_key());
                println!("Wallet Info:");
                println!("Public Key: {}", format_hex(&public_key.to_bytes()));
            }
        },
        Commands::Transaction { action } => {
            match action {
                TransactionCommands::Transfer {
                    key_file,
                    to,
                    amount,
                } => {
                    let key_bytes = std::fs::read(key_file)?;
                    let key_array: [u8; 32] = key_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid key length")?;
                    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_array);
                    let public_key = PublicKey::from(signing_key.verifying_key());
                    let recipient_bytes = hex::decode(to)?;
                    let recipient_array: [u8; 32] = recipient_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid recipient length")?;
                    let recipient_key = PublicKey::from_bytes(&recipient_array)?;
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    let transaction = Transaction {
                        hash: [0u8; 32],
                        sender: public_key,
                        recipient: Address::Wallet(recipient_key),
                        payload: TransactionPayload::Transfer { amount: *amount },
                        nonce: 0, // TODO: Get from chain state
                        timestamp,
                        signature: ed25519_dalek::Signature::from_bytes(&[0u8; 64]).into(),
                        gas_limit: 0,
                        priority: 0,
                        metadata: None,
                    };
                    println!(
                        "Transfer transaction created: {}",
                        format_hex(&transaction.hash)
                    );
                }
                TransactionCommands::Deploy { key_file, contract } => {
                    let key_bytes = std::fs::read(key_file)?;
                    let key_array: [u8; 32] = key_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid key length")?;
                    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_array);
                    let public_key = PublicKey::from(signing_key.verifying_key());
                    let wasm_bytes = std::fs::read(contract)?;
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    let transaction = Transaction {
                        hash: [0u8; 32],
                        sender: public_key,
                        recipient: Address::Contract(ContractId::from_bytes(&[0u8; 32])),
                        payload: TransactionPayload::ContractDeploy { wasm_bytes },
                        nonce: 0, // TODO: Get from chain state
                        timestamp,
                        signature: ed25519_dalek::Signature::from_bytes(&[0u8; 64]).into(),
                        gas_limit: 0,
                        priority: 0,
                        metadata: None,
                    };
                    println!(
                        "Deploy transaction created: {}",
                        format_hex(&transaction.hash)
                    );
                }
                TransactionCommands::Call {
                    key_file,
                    contract_id,
                    method,
                    args,
                } => {
                    let key_bytes = std::fs::read(key_file)?;
                    let key_array: [u8; 32] = key_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid key length")?;
                    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_array);
                    let public_key = PublicKey::from(signing_key.verifying_key());
                    let contract_id_bytes = hex::decode(contract_id)?;
                    let contract_id_array: [u8; 32] = contract_id_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid contract_id length")?;
                    let contract_id = ContractId::from_bytes(&contract_id_array);
                    let args_bytes = args.as_bytes().to_vec();
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    let transaction = Transaction {
                        hash: [0u8; 32],
                        sender: public_key,
                        recipient: Address::Contract(contract_id),
                        payload: TransactionPayload::ContractCall {
                            method: method.clone(),
                            args: args_bytes,
                        },
                        nonce: 0, // TODO: Get from chain state
                        timestamp,
                        signature: ed25519_dalek::Signature::from_bytes(&[0u8; 64]).into(),
                        gas_limit: 0,
                        priority: 0,
                        metadata: None,
                    };
                    println!(
                        "Call transaction created: {}",
                        format_hex(&transaction.hash)
                    );
                }
                TransactionCommands::Data { key_file, data } => {
                    let key_bytes = std::fs::read(key_file)?;
                    let key_array: [u8; 32] = key_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid key length")?;
                    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_array);
                    let public_key = PublicKey::from(signing_key.verifying_key());
                    let data_bytes = hex::decode(data)?;
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    let transaction = Transaction {
                        hash: [0u8; 32],
                        sender: public_key,
                        recipient: Address::Wallet(public_key), // Data tx sent to self
                        payload: TransactionPayload::Data { data: data_bytes },
                        nonce: 0, // TODO: Get from chain state
                        timestamp,
                        signature: ed25519_dalek::Signature::from_bytes(&[0u8; 64]).into(),
                        gas_limit: 0,
                        priority: 0,
                        metadata: None,
                    };
                    println!(
                        "Data transaction created: {}",
                        format_hex(&transaction.hash)
                    );
                }
            }
        }
        Commands::Query { action } => {
            // Use a dummy key for PoAConsensus
            let test_key = PublicKey::from_bytes(&[1u8; 32])?;
            let consensus = PoAConsensus::new(test_key, 1000);
            let storage = SledStorage::new("./data")?;
            let contract_engine = BaaLSContractEngine::new(storage.clone());
            let sync_layer = NoopSync;
            let runtime = Runtime::new(storage, consensus, contract_engine, sync_layer)?;
            match action {
                QueryCommands::Block { height } => match runtime.get_block_by_height(*height)? {
                    Some(block) => {
                        println!("Block {}: {}", height, format_hex(&block.hash));
                        println!("  Timestamp: {}", block.timestamp);
                        println!("  Transactions: {}", block.transactions.len());
                    }
                    None => println!("Block not found"),
                },
                QueryCommands::Account { address } => {
                    let address_bytes = hex::decode(address)?;
                    let address_array: [u8; 32] = address_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid address length")?;
                    let public_key = PublicKey::from_bytes(&address_array)?;
                    match runtime.get_account(&public_key)? {
                        Some(account) => {
                            println!("Account: {}", address);
                            // Print balance if wallet, else print contract info
                            match account {
                                baals::types::Account::Wallet { balance, nonce } => {
                                    println!("  Balance: {}", balance);
                                    println!("  Nonce: {}", nonce);
                                }
                                baals::types::Account::Contract {
                                    code_hash,
                                    storage_root_hash,
                                    nonce,
                                } => {
                                    println!("  Contract code hash: {}", format_hex(&code_hash));
                                    println!(
                                        "  Storage root hash: {}",
                                        format_hex(&storage_root_hash)
                                    );
                                    println!("  Nonce: {}", nonce);
                                }
                            }
                        }
                        None => println!("Account not found"),
                    }
                }
                QueryCommands::Storage { contract_id, key } => {
                    let contract_id_bytes = hex::decode(contract_id)?;
                    let contract_id_array: [u8; 32] = contract_id_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid contract_id length")?;
                    let contract_id = ContractId::from_bytes(&contract_id_array);
                    let key_bytes = hex::decode(key)?;
                    match runtime.contract_storage_read(&contract_id, &key_bytes)? {
                        Some(value) => {
                            println!("Storage value: {}", hex::encode(&value));
                        }
                        None => println!("Storage key not found"),
                    }
                }
                QueryCommands::Contract {
                    contract_id,
                    payload,
                } => {
                    let contract_id_bytes = hex::decode(contract_id)?;
                    let contract_id_array: [u8; 32] = contract_id_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "Invalid contract_id length")?;
                    let contract_id = ContractId::from_bytes(&contract_id_array);
                    let payload_bytes = hex::decode(payload)?;

                    match runtime.contract_engine().query_contract(
                        &contract_id,
                        &payload_bytes,
                        runtime.storage(),
                    ) {
                        Ok(result) => {
                            println!("Query result: {}", hex::encode(&result));
                        }
                        Err(e) => println!("Query error: {}", e),
                    }
                }
            }
        }
        Commands::Dev { action } => {
            match action {
                DevCommands::Start { data_dir } => {
                    println!("Starting BaaLS node with data directory: {:?}", data_dir);
                    let test_key = PublicKey::from_bytes(&[1u8; 32])?;
                    let consensus = PoAConsensus::new(test_key, 1000);
                    let storage = SledStorage::new(data_dir)?;
                    let contract_engine = BaaLSContractEngine::new(storage.clone());
                    let sync_layer = NoopSync;
                    let runtime = Runtime::new(storage, consensus, contract_engine, sync_layer)?;
                    runtime.start()?;
                    println!("Node started successfully");
                }
                DevCommands::GenerateBlock => {
                    println!("Generating test block...");
                    // TODO: Implement block generation
                    println!("Block generation not yet implemented");
                }
                DevCommands::ChainState => {
                    let test_key = PublicKey::from_bytes(&[1u8; 32])?;
                    let consensus = PoAConsensus::new(test_key, 1000);
                    let storage = SledStorage::new("./data")?;
                    let contract_engine = BaaLSContractEngine::new(storage.clone());
                    let sync_layer = NoopSync;
                    let runtime = Runtime::new(storage, consensus, contract_engine, sync_layer)?;
                    let chain_state = runtime.get_chain_state()?;
                    println!("Chain State:");
                    println!("  Height: {}", chain_state.latest_block_index);
                    println!(
                        "  Latest Block: {}",
                        format_hex(&chain_state.latest_block_hash)
                    );
                    println!("  Total Supply: {}", chain_state.total_supply);
                }
            }
        }
    }

    Ok(())
}
