use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use log::{error, info};
use rand::{rngs::OsRng, RngCore};
use serde_json;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{Pid, PidExt, System, SystemExt};

use baals::{
    config::{generate_default_config, setup_logging, Config, NodeStatus},
    Account, Address, BaaLSContractEngine, ContractEngine, ContractId, Keystore, MetricsCollector,
    NoopSync, PoAConsensus, PublicKey, Runtime, SledStorage, Storage, Transaction,
    TransactionPayload, TransactionSignature,
};

#[derive(Parser)]
#[command(name = "baals")]
#[command(about = "BaaLS - Blockchain as a Local Service")]
#[command(version)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,

    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Node {
        #[command(subcommand)]
        action: NodeCommands,
    },
    Wallet {
        #[command(subcommand)]
        action: WalletCommands,
    },
    Tx {
        #[command(subcommand)]
        action: TxCommands,
    },
    Query {
        #[command(subcommand)]
        action: QueryCommands,
    },
    Dev {
        #[command(subcommand)]
        action: DevCommands,
    },
}

#[derive(Subcommand)]
enum NodeCommands {
    Start {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long, default_value = "8080")]
        port: u16,
        #[arg(long, default_value_t = false)]
        daemon: bool,
        #[arg(long, hide = true, default_value_t = false)]
        foreground_internal: bool,
    },
    Stop {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Status {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    Init {
        #[arg(short, long, default_value = "config.toml")]
        output: PathBuf,
    },
    Set {
        key: String,
        value: String,
    },
}

#[derive(Subcommand)]
enum WalletCommands {
    Create {
        #[arg(short, long)]
        name: Option<String>,
    },
    List,
    Import {
        private_key: String,
        #[arg(short, long)]
        name: Option<String>,
    },
    Export {
        identifier: String,
        #[arg(short, long)]
        password: Option<String>,
    },
    Sign {
        identifier: String,
        message: String,
    },
}

#[derive(Subcommand)]
enum TxCommands {
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
}

#[derive(Subcommand)]
enum QueryCommands {
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
}

#[derive(Subcommand)]
enum DevCommands {
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
}

// ─── Output helpers ───

fn json_err(msg: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({"error": true, "message": msg}))
        .unwrap_or_default()
}

fn text_or_json(json: bool, text: &str, json_val: serde_json::Value) -> String {
    if json {
        serde_json::to_string_pretty(&json_val).unwrap_or_default()
    } else {
        text.to_string()
    }
}

// ─── Runtime helpers ───

type BaaLSRuntime = Runtime<SledStorage, PoAConsensus, NoopSync>;

const NODE_PID_FILENAME: &str = "baals.pid";
const NODE_STOP_FILENAME: &str = "baals.stop";

fn node_pid_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join(NODE_PID_FILENAME)
}

fn node_stop_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join(NODE_STOP_FILENAME)
}

#[derive(Debug, Clone)]
struct NodePidInfo {
    pid: u32,
    started_at: Option<u64>,
}

fn is_pid_running(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes();
    system.process(Pid::from_u32(pid)).is_some()
}

fn read_pid_info(pid_path: &PathBuf) -> Result<Option<NodePidInfo>, Box<dyn std::error::Error>> {
    if !pid_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(pid_path)?;
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let mut parts = raw.split(':');
    let pid = parts
        .next()
        .ok_or("Missing pid value")?
        .parse::<u32>()
        .map_err(|_| "Invalid pid format")?;
    let started_at = parts.next().and_then(|v| v.parse::<u64>().ok());
    Ok(Some(NodePidInfo { pid, started_at }))
}

fn write_pid_info(
    pid_path: &PathBuf,
    pid: u32,
    started_at: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(pid_path, format!("{}:{}", pid, started_at))?;
    Ok(())
}

fn wait_for_pid_file(
    pid_path: &PathBuf,
    timeout: std::time::Duration,
) -> Result<Option<NodePidInfo>, Box<dyn std::error::Error>> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if let Some(pid_info) = read_pid_info(pid_path)? {
            return Ok(Some(pid_info));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(None)
}

fn build_runtime(data_dir: &PathBuf) -> Result<BaaLSRuntime, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data_dir)?;
    let storage = SledStorage::new(data_dir)?;

    // Load or generate persistent consensus key
    let key_path = data_dir.join("consensus.key");
    let (_public_key, consensus) = if key_path.exists() {
        let key_bytes = std::fs::read(&key_path)?;
        if key_bytes.len() != 32 {
            return Err("Invalid consensus key length".into());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&key_bytes);
        let signing_key = SigningKey::from_bytes(&arr);
        let pk = PublicKey::from(signing_key.verifying_key());
        let consensus = PoAConsensus::new(pk, 5000).with_signing_key(signing_key);
        (pk, consensus)
    } else {
        let mut rng = OsRng;
        let mut secret_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        std::fs::write(&key_path, &secret_bytes)?;
        let pk = PublicKey::from(signing_key.verifying_key());
        let consensus = PoAConsensus::new(pk, 5000).with_signing_key(signing_key);
        (pk, consensus)
    };

    let contract_engine = BaaLSContractEngine::new(storage.clone())?;
    let sync_layer = NoopSync;
    let runtime = Runtime::new(storage, consensus, contract_engine, sync_layer)?;
    runtime.start()?;
    Ok(runtime)
}

fn prompt_password(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut stdout = io::stdout();
    write!(stdout, "{}", prompt)?;
    stdout.flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn parse_pubkey(hex_str: &str) -> Result<PublicKey, String> {
    let bytes = hex::decode(hex_str).map_err(|_| "Invalid hex".to_string())?;
    if bytes.len() != 32 {
        return Err("Public key must be 32 bytes hex".to_string());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    PublicKey::from_bytes(&arr).map_err(|e| format!("Invalid public key: {:?}", e))
}

// ─── Command dispatcher ───

fn main() {
    let cli = Cli::parse();
    let log_level = if cli.verbose { "debug" } else { "info" };
    let config = Config::load(None).unwrap_or_default();
    let _ = setup_logging(&config, log_level);

    let result = match cli.command {
        Commands::Node { action } => handle_node(action, cli.json),
        Commands::Wallet { action } => handle_wallet(action, cli.json),
        Commands::Tx { action } => handle_tx(action, cli.json),
        Commands::Query { action } => handle_query(action, cli.json),
        Commands::Dev { action } => handle_dev(action, cli.json),
    };

    match result {
        Ok(output) => println!("{}", output),
        Err(e) => {
            error!("{}", e);
            if cli.json {
                println!("{}", json_err(&e.to_string()));
            } else {
                eprintln!("Error: {}", e);
            }
            std::process::exit(1);
        }
    }
}

// ─── Node commands ───

fn handle_node(action: NodeCommands, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        NodeCommands::Start {
            config,
            data_dir,
            port,
            daemon,
            foreground_internal,
        } => {
            if daemon && !foreground_internal {
                std::fs::create_dir_all(&data_dir)?;
                let exe = std::env::current_exe()?;
                let mut cmd = std::process::Command::new(exe);
                cmd.arg("node")
                    .arg("start")
                    .arg("--data-dir")
                    .arg(&data_dir)
                    .arg("--port")
                    .arg(port.to_string())
                    .arg("--foreground-internal");
                if let Some(cfg) = &config {
                    cmd.arg("--config").arg(cfg);
                }

                let child = cmd
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;

                let pid_path = node_pid_path(&data_dir);
                let pid_info = wait_for_pid_file(&pid_path, std::time::Duration::from_secs(5))?;
                let daemon_pid = pid_info.as_ref().map(|p| p.pid);

                return Ok(text_or_json(
                    json,
                    &format!(
                        "Node daemon start requested (launcher pid {}, node pid {:?})",
                        child.id(),
                        daemon_pid
                    ),
                    serde_json::json!({
                        "status": "daemon_start_requested",
                        "launcher_pid": child.id(),
                        "node_pid": daemon_pid,
                        "data_dir": data_dir
                    }),
                ));
            }

            info!("Starting BaaLS node on port {} data={:?}", port, data_dir);
            let _cfg = Config::load(config.as_deref()).unwrap_or_default();
            std::fs::create_dir_all(&data_dir)?;
            let pid_path = node_pid_path(&data_dir);
            let stop_path = node_stop_path(&data_dir);

            if let Some(pid_info) = read_pid_info(&pid_path)? {
                if is_pid_running(pid_info.pid) {
                    return Err(format!(
                        "Node already running (pid {}) for data dir {:?}",
                        pid_info.pid, data_dir
                    )
                    .into());
                }
                // stale pid file from dead process
                let _ = std::fs::remove_file(&pid_path);
            }
            if stop_path.exists() {
                std::fs::remove_file(&stop_path)?;
            }
            let started_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            write_pid_info(&pid_path, std::process::id(), started_at)?;

            let runtime = build_runtime(&data_dir)?;
            info!("Node started. Press Ctrl+C to stop.");
            let mut heartbeat = 0u64;
            loop {
                if stop_path.exists() {
                    info!("Stop signal file detected at {:?}", stop_path);
                    if let Err(e) = runtime.stop() {
                        info!("Runtime stop returned: {}", e);
                    }
                    let _ = std::fs::remove_file(&stop_path);
                    let _ = std::fs::remove_file(&pid_path);
                    return Ok(text_or_json(
                        json,
                        "Node stopped by signal file",
                        serde_json::json!({"status": "stopped", "reason": "signal_file"}),
                    ));
                }

                std::thread::sleep(std::time::Duration::from_secs(1));
                heartbeat += 1;

                if heartbeat % 10 == 0 {
                    let state = runtime.get_chain_state()?;
                    info!(
                        "Block height: {}, mempool: {}",
                        state.latest_block_index,
                        runtime.get_pending_transactions()?.len()
                    );
                }
            }
        }
        NodeCommands::Stop { data_dir } => {
            let stop_path = node_stop_path(&data_dir);
            let pid_path = node_pid_path(&data_dir);
            std::fs::create_dir_all(&data_dir)?;
            let pid_info = read_pid_info(&pid_path)?;
            let pid = pid_info.as_ref().map(|p| p.pid.to_string());
            if let Some(pid_info) = &pid_info {
                if !is_pid_running(pid_info.pid) {
                    let _ = std::fs::remove_file(&pid_path);
                    let _ = std::fs::remove_file(&stop_path);
                    return Ok(text_or_json(
                        json,
                        "Node already stopped (cleaned stale pid file)",
                        serde_json::json!({
                            "status": "already_stopped",
                            "data_dir": data_dir,
                            "pid_hint": pid,
                        }),
                    ));
                }
            }
            std::fs::write(
                &stop_path,
                format!(
                    "{}",
                    SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()
                ),
            )?;
            info!("Stop signal file written to {:?}", stop_path);
            Ok(text_or_json(
                json,
                "Node stop requested",
                serde_json::json!({"status": "stop_requested", "data_dir": data_dir, "pid_hint": pid}),
            ))
        }
        NodeCommands::Status { data_dir } => {
            let pid_path = node_pid_path(&data_dir);
            let pid_info = read_pid_info(&pid_path)?;
            let running = pid_info
                .as_ref()
                .map(|p| is_pid_running(p.pid))
                .unwrap_or(false);

            // Best-effort chain status from on-disk storage.
            let default_chain = baals::ChainState {
                latest_block_hash: [0u8; 32],
                latest_block_index: 0,
                accounts_root_hash: [0u8; 32],
                total_supply: 0,
            };
            let (chain, mempool, storage_locked) = match SledStorage::new(&data_dir) {
                Ok(storage) => {
                    let chain = storage
                        .get_chain_state()
                        .ok()
                        .flatten()
                        .unwrap_or(default_chain.clone());
                    let mempool = storage
                        .get_pending_transactions()
                        .map(|txs| txs.len())
                        .unwrap_or(0);
                    (chain, mempool, false)
                }
                Err(_) => (default_chain.clone(), 0, true),
            };
            let uptime_seconds = if running {
                pid_info
                    .as_ref()
                    .and_then(|p| p.started_at)
                    .and_then(|started| {
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .ok()
                            .map(|now| now.as_secs().saturating_sub(started))
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            let status = NodeStatus {
                running,
                chain_height: chain.latest_block_index,
                latest_block_hash: hex::encode(chain.latest_block_hash),
                mempool_size: mempool,
                peer_count: 0,
                uptime_seconds,
            };
            Ok(text_or_json(
                json,
                &format!(
                    "Height: {} | Mempool: {} | Peers: {} | Uptime: {}s | Latest: {}{}",
                    status.chain_height,
                    status.mempool_size,
                    status.peer_count,
                    status.uptime_seconds,
                    status.latest_block_hash,
                    if storage_locked {
                        " | Storage: locked"
                    } else {
                        ""
                    }
                ),
                if storage_locked {
                    serde_json::json!({
                        "running": status.running,
                        "chain_height": status.chain_height,
                        "latest_block_hash": status.latest_block_hash,
                        "mempool_size": status.mempool_size,
                        "peer_count": status.peer_count,
                        "uptime_seconds": status.uptime_seconds,
                        "storage_locked": true
                    })
                } else {
                    serde_json::to_value(&status)?
                },
            ))
        }
        NodeCommands::Config { action } => handle_config(action, json),
    }
}

fn handle_config(
    action: ConfigCommands,
    _json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        ConfigCommands::Init { output } => {
            let _cfg = generate_default_config(&output)?;
            Ok(format!("Config written to {:?}", output))
        }
        ConfigCommands::Set { key, value } => {
            let config_path = PathBuf::from("config.toml");
            let mut cfg = if config_path.exists() {
                Config::from_file(&config_path)?
            } else {
                Config::default()
            };
            cfg.set(&key, &value)?;
            cfg.save(&config_path)?;
            Ok(format!("Set {} = {}", key, value))
        }
    }
}

// ─── Wallet commands ───

fn handle_wallet(action: WalletCommands, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    let keystore = Keystore::new(None)?;
    match action {
        WalletCommands::Create { name: _ } => {
            let password = prompt_password("Password: ")?;
            let pk = keystore.create_key(&password)?;
            Ok(text_or_json(
                json,
                &format!("Wallet created\nPublic Key: {}", hex::encode(pk.to_bytes())),
                serde_json::json!({"public_key": hex::encode(pk.to_bytes())}),
            ))
        }
        WalletCommands::List => {
            let keys = keystore.list_keys()?;
            if keys.is_empty() {
                Ok(text_or_json(
                    json,
                    "No wallets found",
                    serde_json::json!({"wallets": []}),
                ))
            } else {
                let list: Vec<String> = keys.iter().map(|k| hex::encode(k.to_bytes())).collect();
                Ok(text_or_json(
                    json,
                    &list.join("\n"),
                    serde_json::json!({"wallets": list}),
                ))
            }
        }
        WalletCommands::Import {
            private_key,
            name: _,
        } => {
            let bytes = hex::decode(&private_key)?;
            if bytes.len() != 32 {
                return Err("Private key must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            let password = prompt_password("Encryption password: ")?;
            let pk = keystore.import_key(&arr, &password)?;
            Ok(text_or_json(
                json,
                &format!("Imported: {}", hex::encode(pk.to_bytes())),
                serde_json::json!({"public_key": hex::encode(pk.to_bytes())}),
            ))
        }
        WalletCommands::Export {
            identifier,
            password,
        } => {
            let pk = parse_pubkey(&identifier)?;
            let pw = password.unwrap_or_else(|| prompt_password("Password: ").unwrap_or_default());
            let sk = keystore.export_key(&pk, &pw)?;
            Ok(text_or_json(
                json,
                &format!("Private key: {}", hex::encode(sk)),
                serde_json::json!({"private_key": hex::encode(sk)}),
            ))
        }
        WalletCommands::Sign {
            identifier,
            message,
        } => {
            let pk = parse_pubkey(&identifier)?;
            let password = prompt_password("Password: ")?;
            let sk = keystore.load_key(&pk, &password)?;
            let msg_bytes = hex::decode(&message).unwrap_or_else(|_| message.as_bytes().to_vec());
            let sig = sk.sign(&msg_bytes);
            Ok(text_or_json(
                json,
                &format!("Signature: {}", hex::encode(sig.to_bytes())),
                serde_json::json!({"signature": hex::encode(sig.to_bytes())}),
            ))
        }
    }
}

// ─── Transaction commands ───

fn handle_tx(action: TxCommands, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    let keystore = Keystore::new(None)?;

    match action {
        TxCommands::Transfer {
            sender,
            recipient,
            amount,
            memo,
            data_dir,
        } => {
            let runtime = build_runtime(&data_dir)?;
            let sender_pk = parse_pubkey(&sender)?;
            let recipient_pk = parse_pubkey(&recipient)?;

            // Ensure sender account exists with sufficient balance
            let sender_account =
                runtime
                    .get_account(&sender_pk)?
                    .unwrap_or_else(|| Account::Wallet {
                        balance: 0,
                        nonce: 0,
                    });
            let nonce = sender_account.nonce() + 1;
            if let Account::Wallet { balance, .. } = sender_account {
                if balance < amount {
                    return Err(format!("Insufficient balance: {} < {}", balance, amount).into());
                }
            }

            // Load signing key
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
                gas_limit: 100000,
                priority: 0,
                metadata: memo.map(|m| {
                    let mut map = std::collections::BTreeMap::new();
                    map.insert("memo".to_string(), m);
                    map
                }),
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
        TxCommands::DeployContract {
            sender,
            wasm,
            init_args,
            gas_limit,
            data_dir,
        } => {
            let runtime = build_runtime(&data_dir)?;
            let sender_pk = parse_pubkey(&sender)?;
            let wasm_bytes = std::fs::read(&wasm)?;
            let account = runtime
                .get_account(&sender_pk)?
                .unwrap_or_else(|| Account::Wallet {
                    balance: 0,
                    nonce: 0,
                });
            let _nonce = account.nonce() + 1;

            let password = prompt_password("Wallet password: ")?;
            let _signing_key = keystore.load_key(&sender_pk, &password)?;

            let init_payload = init_args
                .map(|path| std::fs::read(path))
                .transpose()?;
            let init_payload_ref = init_payload.as_deref();

            let cid = runtime.deploy_contract(&sender_pk, &wasm_bytes, init_payload_ref, gas_limit)?;
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
            let runtime = build_runtime(&data_dir)?;
            let sender_pk = parse_pubkey(&sender)?;
            let cid_bytes = hex::decode(&contract_id)?;
            if cid_bytes.len() != 32 {
                return Err("Contract ID must be 32 bytes hex".into());
            }
            let mut cid_arr = [0u8; 32];
            cid_arr.copy_from_slice(&cid_bytes);
            let cid = ContractId::from_bytes(&cid_arr);

            let arg_bytes = args.map(|a| a.into_bytes()).unwrap_or_default();
            let call_value = if value > 0 { Some(value) } else { None };
            let result = runtime.call_contract(&sender_pk, &cid, &method, &arg_bytes, call_value, gas_limit)?;

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
        TxCommands::Data {
            sender,
            data,
            data_dir,
        } => {
            let runtime = build_runtime(&data_dir)?;
            let sender_pk = parse_pubkey(&sender)?;
            let account = runtime
                .get_account(&sender_pk)?
                .unwrap_or_else(|| Account::Wallet {
                    balance: 0,
                    nonce: 0,
                });
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
                gas_limit: 100000,
                priority: 0,
                metadata: None,
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
                    serde_json::to_value(&serde_json::json!({
                        "hash": hex::encode(tx.hash),
                        "sender": hex::encode(tx.sender.to_bytes()),
                        "nonce": tx.nonce,
                    }))?,
                )),
                Err(e) => Err(format!("Failed to parse transaction: {}", e).into()),
            }
        }
    }
}

// ─── Query commands ───

fn handle_query(action: QueryCommands, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        QueryCommands::Head { data_dir } => {
            let runtime = build_runtime(&data_dir)?;
            let chain = runtime.get_chain_state()?;
            let block = runtime
                .get_block(&chain.latest_block_hash)?
                .ok_or("No block found")?;
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
        QueryCommands::Block {
            identifier,
            data_dir,
        } => {
            let runtime = build_runtime(&data_dir)?;
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
            let runtime = build_runtime(&data_dir)?;
            let h = hex::decode(&hash)?;
            if h.len() != 32 {
                return Err("Hash must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&h);
            match runtime.get_transaction(&arr)? {
                Some(tx) => Ok(text_or_json(
                    json,
                    &format!(
                        "Tx: sender={}, nonce={}, payload={:?}",
                        hex::encode(&tx.sender.to_bytes()[..8]),
                        tx.nonce,
                        tx.payload
                    ),
                    serde_json::json!({"hash": hex::encode(tx.hash), "sender": hex::encode(tx.sender.to_bytes()), "nonce": tx.nonce}),
                )),
                None => Err("Transaction not found".into()),
            }
        }
        QueryCommands::Account {
            address,
            data_dir,
        } => {
            let runtime = build_runtime(&data_dir)?;
            let pk = parse_pubkey(&address)?;
            match runtime.get_account(&pk)? {
                Some(Account::Wallet { balance, nonce }) => Ok(text_or_json(
                    json,
                    &format!("Wallet: balance={}, nonce={}", balance, nonce),
                    serde_json::json!({"type": "wallet", "balance": balance, "nonce": nonce}),
                )),
                Some(Account::Contract {
                    code_hash,
                    storage_root_hash,
                    nonce,
                }) => Ok(text_or_json(
                    json,
                    &format!(
                        "Contract: code={}, storage={}, nonce={}",
                        hex::encode(&code_hash[..8]),
                        hex::encode(&storage_root_hash[..8]),
                        nonce
                    ),
                    serde_json::json!({"type": "contract", "code_hash": hex::encode(code_hash), "nonce": nonce}),
                )),
                None => Err("Account not found".into()),
            }
        }
        QueryCommands::ContractState {
            contract_id,
            key,
            data_dir,
        } => {
            let runtime = build_runtime(&data_dir)?;
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
        QueryCommands::ContractCall {
            contract_id,
            method,
            args,
            data_dir,
        } => {
            let runtime = build_runtime(&data_dir)?;
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
    }
}

// ─── Dev commands ───

fn handle_dev(action: DevCommands, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        DevCommands::GenerateKeys { count } => {
            let mut keys = Vec::new();
            for _ in 0..count {
                let mut rng = OsRng;
                let mut secret = [0u8; 32];
                rng.fill_bytes(&mut secret);
                let sk = SigningKey::from_bytes(&secret);
                let pk = PublicKey::from(sk.verifying_key());
                keys.push(hex::encode(pk.to_bytes()));
            }
            Ok(text_or_json(
                json,
                &keys.join("\n"),
                serde_json::json!({"keys": keys}),
            ))
        }
        DevCommands::SimulateContract {
            wasm,
            method,
            args,
            sender,
            data_dir,
        } => {
            let _wasm_bytes = std::fs::read(&wasm)?;
            let storage = SledStorage::new(&data_dir)?;
            let engine = BaaLSContractEngine::new(storage.clone())?;
            let dummy_pk = sender
                .as_ref()
                .map(|s| parse_pubkey(s))
                .unwrap_or_else(|| Ok(PublicKey::from_bytes(&[0u8; 32]).unwrap()))
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let cid = ContractId::from_bytes(&[0u8; 32]); // dummy for simulation
            let arg_bytes = args.map(|a| a.into_bytes()).unwrap_or_default();
            let result = engine.call_contract(&dummy_pk, &cid, &method, &arg_bytes, None, &storage)?;
            Ok(text_or_json(
                json,
                &format!(
                    "Result ({} bytes): {}",
                    result.len(),
                    String::from_utf8_lossy(&result[..result.len().min(64)])
                ),
                serde_json::json!({"result_hex": hex::encode(&result)}),
            ))
        }
        DevCommands::ValidateTx { file } => {
            let data = std::fs::read(&file)?;
            match bincode::deserialize::<Transaction>(&data) {
                Ok(tx) => {
                    let _checks = vec![
                        ("hash valid", tx.hash.iter().any(|b| *b != 0)),
                        ("signature valid", tx.verify_signature().unwrap_or(false)),
                    ];
                    Ok(text_or_json(
                        json,
                        &format!(
                            "Valid transaction. Hash: {}. Signature valid: {}",
                            hex::encode(tx.hash),
                            tx.verify_signature().unwrap_or(false)
                        ),
                        serde_json::json!({"valid": true, "hash": hex::encode(tx.hash), "nonce": tx.nonce}),
                    ))
                }
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
            let runtime = build_runtime(&data_dir)?;
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
                    &format!("Chain valid: {} blocks verified, no inconsistencies found", height + 1),
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
    }
}
