use clap::Subcommand;
use log::{error, info};
use rand::RngCore;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::{
    config::{Config, NodeStatus, StorageBackend},
    Account, AnyStorage, BaaLSContractEngine, CustomSync, Keystore, NoopSync, PoAConsensus,
    PublicKey, RedbStorage, Runtime, SledStorage, Storage, SyncLayer, SyncWrapper, TlsConfig,
    Transaction,
};

use crate::cli::{parse_pubkey, text_or_json};

#[derive(Subcommand)]
pub enum NodeCommands {
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
        #[arg(long)]
        peer: Vec<String>,
        #[arg(long, default_value = "0.0.0.0:9070")]
        listen: String,
        #[arg(long)]
        mdns: bool,
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
    Backup {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long, default_value = "backup.baals")]
        output: PathBuf,
    },
    Restore {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long)]
        input: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    Init {
        #[arg(short, long, default_value = "config.toml")]
        output: PathBuf,
    },
    Set {
        key: String,
        value: String,
    },
}

pub type BaaLSRuntime = Runtime<AnyStorage, PoAConsensus, SyncWrapper>;

const NODE_STOP_FILENAME: &str = "baals.stop";

fn node_pid_path(data_dir: &Path) -> PathBuf {
    data_dir.join("baals.pid")
}

fn node_stop_path(data_dir: &Path) -> PathBuf {
    data_dir.join(NODE_STOP_FILENAME)
}

#[derive(Debug, Clone)]
struct NodePidInfo {
    pid: u32,
    started_at: Option<u64>,
}

fn is_pid_running(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
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
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(libc::O_EXCL)
            .open(pid_path)
        {
            file.write_all(format!("{}:{}", pid, started_at).as_bytes())?;
            file.sync_all()?;
        } else {
            if let Ok(Some(info)) = read_pid_info(pid_path) {
                if is_pid_running(info.pid) {
                    return Err(
                        "Another BaaLS instance appears to be running (PID file locked)".into()
                    );
                }
                std::fs::write(pid_path, format!("{}:{}", pid, started_at))?;
            } else {
                std::fs::write(pid_path, format!("{}:{}", pid, started_at))?;
            }
        }
    }
    #[cfg(not(unix))]
    {
        std::fs::write(pid_path, format!("{}:{}", pid, started_at))?;
    }
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

pub fn build_runtime(
    data_dir: &PathBuf,
    config: &Config,
    peers: &[String],
    listen_addr: &str,
    mdns: bool,
) -> Result<(BaaLSRuntime, PublicKey), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data_dir)?;
    log::warn!(
        "BaaLS stores ledger data in cleartext at {:?}. \
         In production, ensure this path resides on an encrypted volume \
         (e.g. LUKS, BitLocker, or cloud KMS-backed storage).",
        data_dir
    );
    let storage: AnyStorage = match config.storage.backend {
        StorageBackend::Redb => {
            AnyStorage::Redb(RedbStorage::new(data_dir).map_err(|e| e.to_string())?)
        }
        _ => AnyStorage::Sled(SledStorage::new(data_dir)?),
    };

    let (public_key, consensus, signing_key) = {
        let signing_key: ed25519_dalek::SigningKey;
        if let Ok(raw_hex) = std::env::var("BAALS_CONSENSUS_KEY") {
            let raw_hex = raw_hex.trim().to_string();
            if raw_hex.is_empty() {
                return Err("BAALS_CONSENSUS_KEY env var is set but empty".into());
            }
            let key_bytes = hex::decode(&raw_hex)
                .map_err(|_| "BAALS_CONSENSUS_KEY: invalid hex encoding".to_string())?;
            if key_bytes.len() != 32 {
                return Err("BAALS_CONSENSUS_KEY: must be 32 bytes (64 hex chars)".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&key_bytes);
            signing_key = ed25519_dalek::SigningKey::from_bytes(&arr);
            info!("Loaded consensus key from BAALS_CONSENSUS_KEY environment variable");
        } else if let Ok(password) = std::env::var("BAALS_CONSENSUS_PASSWORD") {
            let enc_path = Keystore::consensus_key_path(data_dir);
            if enc_path.exists() {
                let sk = Keystore::load_consensus_key(data_dir, &password)
                    .map_err(|e| format!("Failed to load encrypted consensus key: {}", e))?;
                signing_key = sk;
            } else {
                let sk = Keystore::generate_consensus_key(data_dir, &password)
                    .map_err(|e| format!("Failed to generate encrypted consensus key: {}", e))?;
                signing_key = sk;
            }
        } else {
            let key_path = data_dir.join("consensus.key");
            if key_path.exists() {
                let key_bytes = std::fs::read(&key_path)?;
                if key_bytes.len() != 32 {
                    return Err("Invalid consensus key length".into());
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&key_bytes);
                signing_key = ed25519_dalek::SigningKey::from_bytes(&arr);
                log::warn!(
                    "Consensus key loaded in cleartext from {:?}. \
                     Set BAALS_CONSENSUS_PASSWORD env var and delete this file \
                     to use encrypted key storage (AES-256-GCM + Argon2id).",
                    key_path
                );
            } else {
                let mut sk_bytes = [0u8; 32];
                rand::rng().fill_bytes(&mut sk_bytes);
                signing_key = ed25519_dalek::SigningKey::from_bytes(&sk_bytes);
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&key_path)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                }
                file.write_all(&sk_bytes)?;
                log::warn!(
                    "Generated new UNENCRYPTED consensus key at {:?}. \
                     Set BAALS_CONSENSUS_PASSWORD env var and delete this file \
                     to use encrypted key storage (AES-256-GCM + Argon2id).",
                    key_path
                );
            }
        }
        let pk = PublicKey::from(signing_key.verifying_key());
        let consensus = PoAConsensus::new(pk, config.consensus.block_time_ms)
            .with_signing_key(signing_key.clone());
        (pk, consensus, signing_key)
    };

    let mut consensus = consensus;
    consensus.load_authorized_signers_from_storage(&storage)?;

    let contract_engine = BaaLSContractEngine::new(storage.clone())?;

    let listen_socket: std::net::SocketAddr =
        listen_addr.parse().unwrap_or_else(|_| "0.0.0.0:9070".parse().unwrap());
    let sync_layer = if peers.is_empty() && !mdns {
        SyncWrapper::Noop(NoopSync)
    } else {
        let mut cs = CustomSync::new(public_key, listen_socket)
            .with_signing_key(signing_key)
            .with_storage(storage.clone_storage());
        if config.network.tls_enabled {
            let tls = TlsConfig::load(
                &config.network.tls_cert_path,
                &config.network.tls_key_path,
                Some(config.network.tls_ca_cert_path.as_str()).filter(|s| !s.is_empty()),
            )?;
            cs = cs.with_tls(tls);
        }
        if !config.network.allowed_peers.is_empty() {
            use std::collections::HashSet;
            let mut peers_set = HashSet::new();
            for peer_hex in &config.network.allowed_peers {
                let bytes = hex::decode(peer_hex)?;
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                peers_set.insert(PublicKey::from_bytes(&arr)?);
            }
            cs = cs.with_authorized_peers(peers_set);
        }
        for peer_addr in peers {
            if let Err(e) = cs.add_peer_by_address(peer_addr) {
                log::warn!("Failed to add peer '{}': {}", peer_addr, e);
            }
        }
        SyncWrapper::Custom(Box::new(cs))
    };

    let mut runtime = Runtime::new(storage, consensus, contract_engine, sync_layer)?;
    runtime.auto_block_interval_ms = config.consensus.block_time_ms;
    runtime.auto_block_mempool_threshold = 10;
    runtime.min_gas_price = config.consensus.min_gas_price;
    runtime.chain_id = config.consensus.chain_id;
    runtime.finality_depth = config.consensus.finality_depth;
    runtime.start()?;
    Ok((runtime, public_key))
}

struct RateLimiter {
    requests: std::collections::HashMap<String, Vec<std::time::Instant>>,
    max_requests: usize,
    window_secs: u64,
}

impl RateLimiter {
    fn new(max_requests: usize, window_secs: u64) -> Self {
        Self { requests: std::collections::HashMap::new(), max_requests, window_secs }
    }

    fn check_and_record(&mut self, ip: &str) -> bool {
        let now = std::time::Instant::now();
        let window = std::time::Duration::from_secs(self.window_secs);
        let entries = self.requests.entry(ip.to_string()).or_default();
        entries.retain(|t| now.duration_since(*t) < window);
        if entries.len() >= self.max_requests {
            false
        } else {
            entries.push(now);
            true
        }
    }
}

fn spawn_health_server(
    runtime: BaaLSRuntime,
    bind_addr: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let server = Server::http(&bind_addr).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            format!("failed to bind health endpoint on {}: {}", bind_addr, e),
        )
    })?;

    std::thread::spawn(move || {
        let admin_token =
            std::env::var("BAALS_ADMIN_TOKEN").ok().filter(|token| !token.trim().is_empty());

        fn is_authorized_request(
            request: &tiny_http::Request,
            expected_token: Option<&str>,
        ) -> bool {
            if let Some(token) = expected_token {
                let expected = format!("Bearer {}", token);
                request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.as_str() == expected)
                    .unwrap_or(false)
            } else {
                true
            }
        }

        fn respond_json(request: tiny_http::Request, status: u16, body: String) {
            let mut response = Response::from_string(body).with_status_code(StatusCode(status));
            if let Ok(header) =
                Header::from_bytes(b"Content-Type".as_slice(), b"application/json".as_slice())
            {
                response = response.with_header(header);
            }
            let _ = request.respond(response);
        }

        let mut rate_limiter = RateLimiter::new(10, 1);
        info!("Health endpoint listening on http://{}/health", bind_addr);
        while runtime.is_running() {
            match server.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(Some(mut request)) => {
                    let request_url = request.url().to_string();
                    let request_url_str = request_url.as_str();
                    let client_ip =
                        request.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
                    if !rate_limiter.check_and_record(&client_ip) {
                        let _ = request.respond(
                            Response::from_string("Too Many Requests")
                                .with_status_code(StatusCode(429)),
                        );
                        continue;
                    }

                    let is_loopback =
                        request.remote_addr().map(|addr| addr.ip().is_loopback()).unwrap_or(false);
                    let is_mutating_endpoint = request.method() == &Method::Post
                        && matches!(
                            request_url_str,
                            "/tx/submit"
                                | "/api/v1/transactions"
                                | "/account"
                                | "/api/v1/accounts"
                                | "/contract/deploy"
                                | "/api/v1/contracts/deploy"
                                | "/contract/call"
                                | "/api/v1/contracts/invoke"
                        );
                    if is_mutating_endpoint && !is_loopback {
                        respond_json(
                            request,
                            403,
                            serde_json::json!({
                                "error": "mutating endpoints are restricted to loopback clients"
                            })
                            .to_string(),
                        );
                        continue;
                    }
                    if is_mutating_endpoint {
                        let Some(expected_token) = admin_token.as_deref() else {
                            respond_json(
                                request,
                                503,
                                serde_json::json!({
                                    "error": "mutating endpoints disabled until BAALS_ADMIN_TOKEN is configured"
                                })
                                .to_string(),
                            );
                            continue;
                        };
                        if !is_authorized_request(&request, Some(expected_token)) {
                            respond_json(
                                request,
                                401,
                                serde_json::json!({
                                    "error": "missing or invalid Authorization header"
                                })
                                .to_string(),
                            );
                            continue;
                        }
                    }

                    let is_health = request.method() == &Method::Get
                        && matches!(request_url_str, "/health" | "/api/v1/health");
                    if is_health {
                        match runtime.get_health_status() {
                            Ok(health) => {
                                let body = serde_json::to_string(&health)
                                    .unwrap_or_else(|_| "{\"status\":\"unhealthy\"}".to_string());
                                respond_json(request, 200, body);
                            }
                            Err(e) => {
                                let body = serde_json::json!({
                                    "status": "unhealthy",
                                    "error": e.to_string()
                                })
                                .to_string();
                                respond_json(request, 500, body);
                            }
                        }
                    } else if request.method() == &Method::Get
                        && request_url_str.starts_with("/proof/account/")
                    {
                        let addr_hex = &request_url_str["/proof/account/".len()..];
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let pk = parse_pubkey(addr_hex)?;
                                let _account =
                                    runtime.get_account(&pk)?.ok_or("Account not found")?;
                                let mut smt = crate::SparseMerkleTree::new();
                                let all_accounts = runtime.storage().get_all_accounts()?;
                                for (addr, acct) in &all_accounts {
                                    let bytes = bincode::serialize(acct)?;
                                    smt.insert(addr.to_bytes(), bytes);
                                }
                                let chain_state = runtime.get_chain_state()?;
                                if smt.root() != chain_state.accounts_root_hash {
                                    log::warn!(
                                        "Account SMT root mismatch: computed={}, stored={}",
                                        hex::encode(smt.root()),
                                        hex::encode(chain_state.accounts_root_hash)
                                    );
                                }
                                let proof = smt.generate_proof(pk.to_bytes());
                                Ok(serde_json::json!({
                                    "root": hex::encode(proof.root),
                                    "proof": proof.proof.iter().map(hex::encode).collect::<Vec<_>>(),
                                    "key": hex::encode(proof.key),
                                    "value_hex": hex::encode(&proof.value),
                                }))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (404, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Get
                        && request_url_str.starts_with("/proof/contract/")
                    {
                        let path = &request_url_str["/proof/contract/".len()..];
                        let parts: Vec<&str> = path.split("/storage/").collect();
                        if parts.len() == 2 {
                            let cid_hex = parts[0];
                            let key_hex = parts[1];
                            let response_json =
                                (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                    let mut smt = crate::SparseMerkleTree::new();
                                    let cid_bytes = hex::decode(cid_hex)
                                        .map_err(|e| format!("Invalid contract ID hex: {}", e))?;
                                    let cid_arr: [u8; 32] = cid_bytes
                                        .try_into()
                                        .map_err(|_| "Contract ID must be 32 bytes")?;
                                    let all_storage = runtime
                                        .storage()
                                        .contract_storage_read_all(&crate::ContractId::from_bytes(
                                            &cid_arr,
                                        ))
                                        .map_err(|e| {
                                            format!("Failed to read contract storage: {}", e)
                                        })?;
                                    for (k, v) in all_storage {
                                        let mut k_arr = [0u8; 32];
                                        let len = k.len().min(32);
                                        k_arr[..len].copy_from_slice(&k[..len]);
                                        smt.insert(k_arr, v);
                                    }
                                    let mut key_arr = [0u8; 32];
                                    let decoded_key = hex::decode(key_hex).unwrap_or_default();
                                    let copy_len = decoded_key.len().min(32);
                                    key_arr[..copy_len].copy_from_slice(&decoded_key[..copy_len]);
                                    let proof = smt.generate_proof(key_arr);
                                    Ok(serde_json::json!({
                                        "root": hex::encode(proof.root),
                                        "proof": proof.proof.iter().map(hex::encode).collect::<Vec<_>>(),
                                        "key": hex::encode(proof.key),
                                        "value_hex": hex::encode(&proof.value),
                                    }))
                                })();
                            let (status, body) = match response_json {
                                Ok(json) => (200, json.to_string()),
                                Err(e) => {
                                    (404, serde_json::json!({"error": e.to_string()}).to_string())
                                }
                            };
                            respond_json(request, status, body);
                        } else {
                            let _ = request.respond(
                                Response::from_string("Not Found")
                                    .with_status_code(StatusCode(404)),
                            );
                        }
                    } else if request.method() == &Method::Post
                        && matches!(request_url_str, "/tx/submit" | "/api/v1/transactions")
                    {
                        let mut body = String::new();
                        let _ = request.as_reader().read_to_string(&mut body);
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let tx: Transaction = serde_json::from_str(&body)?;
                                runtime.submit_transaction(tx)?;
                                Ok(serde_json::json!({"status": "ok"}))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (400, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Post
                        && matches!(request_url_str, "/account" | "/api/v1/accounts")
                    {
                        let mut body = String::new();
                        let _ = request.as_reader().read_to_string(&mut body);
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let v: serde_json::Value = serde_json::from_str(&body)?;
                                let pubkey_hex =
                                    v["pubkey"].as_str().ok_or("Missing 'pubkey' field")?;
                                let balance = v["balance"].as_u64().unwrap_or(0);
                                let pk = parse_pubkey(pubkey_hex)?;
                                let account = Account::Wallet { balance, nonce: 0 };
                                runtime.create_account(&pk, account)?;
                                Ok(serde_json::json!({"status": "ok"}))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (400, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Post
                        && matches!(
                            request_url_str,
                            "/contract/deploy" | "/api/v1/contracts/deploy"
                        )
                    {
                        let mut body = String::new();
                        let _ = request.as_reader().read_to_string(&mut body);
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let v: serde_json::Value = serde_json::from_str(&body)?;
                                let deployer_hex =
                                    v["deployer_hex"].as_str().ok_or("Missing 'deployer_hex'")?;
                                let wasm_hex =
                                    v["wasm_hex"].as_str().ok_or("Missing 'wasm_hex'")?;
                                let init_hex = v["init_hex"].as_str().unwrap_or("");
                                let gas_limit = v["gas_limit"].as_u64().unwrap_or(1_000_000);
                                let deployer = parse_pubkey(deployer_hex)?;
                                let wasm_bytes = hex::decode(wasm_hex)
                                    .map_err(|e| format!("Invalid wasm hex: {}", e))?;
                                let init_payload: Option<Vec<u8>> = if init_hex.is_empty() {
                                    None
                                } else {
                                    Some(
                                        hex::decode(init_hex)
                                            .map_err(|e| format!("Invalid init hex: {}", e))?,
                                    )
                                };
                                let cid = runtime.deploy_contract(
                                    &deployer,
                                    &wasm_bytes,
                                    init_payload.as_deref(),
                                    gas_limit,
                                )?;
                                Ok(serde_json::json!({"contract_id": hex::encode(cid.to_bytes())}))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (400, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Post
                        && matches!(request_url_str, "/contract/call" | "/api/v1/contracts/invoke")
                    {
                        let mut body = String::new();
                        let _ = request.as_reader().read_to_string(&mut body);
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let v: serde_json::Value = serde_json::from_str(&body)?;
                                let caller_hex =
                                    v["caller_hex"].as_str().ok_or("Missing 'caller_hex'")?;
                                let cid_hex =
                                    v["contract_id"].as_str().ok_or("Missing 'contract_id'")?;
                                let method = v["method"].as_str().ok_or("Missing 'method'")?;
                                let value = v["value"].as_u64();
                                let caller = parse_pubkey(caller_hex)?;
                                let cid_bytes = hex::decode(cid_hex)
                                    .map_err(|e| format!("Invalid contract id: {}", e))?;
                                if cid_bytes.len() != 32 {
                                    return Err("Contract id must be 32 bytes".into());
                                }
                                let mut cid_arr = [0u8; 32];
                                cid_arr.copy_from_slice(&cid_bytes);
                                let cid = crate::ContractId::from_bytes(&cid_arr);

                                let args: Vec<Vec<u8>> = if let Some(arr) = v["args"].as_array() {
                                    arr.iter()
                                        .map(|a| {
                                            hex::decode(a.as_str().unwrap_or(""))
                                                .map_err(|e| format!("Invalid arg hex: {}", e))
                                        })
                                        .collect::<Result<_, _>>()?
                                } else {
                                    Vec::new()
                                };

                                let wrapped_value = value.filter(|&val| val > 0);
                                let gas_limit = v["gas_limit"].as_u64().unwrap_or(1_000_000);
                                let result = runtime.call_contract(
                                    &caller,
                                    &cid,
                                    method,
                                    &args,
                                    wrapped_value,
                                    gas_limit,
                                )?;
                                Ok(serde_json::json!({
                                    "result_hex": hex::encode(&result),
                                    "result_len": result.len(),
                                }))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (400, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Post
                        && matches!(request_url_str, "/contract/query" | "/api/v1/contracts/call")
                    {
                        let mut body = String::new();
                        let _ = request.as_reader().read_to_string(&mut body);
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let v: serde_json::Value = serde_json::from_str(&body)?;
                                let cid_hex =
                                    v["contract_id"].as_str().ok_or("Missing 'contract_id'")?;
                                let method = v["method"].as_str().ok_or("Missing 'method'")?;
                                let payload_hex = v["payload_hex"].as_str().unwrap_or("");
                                let cid_bytes = hex::decode(cid_hex)
                                    .map_err(|e| format!("Invalid contract id: {}", e))?;
                                if cid_bytes.len() != 32 {
                                    return Err("Contract id must be 32 bytes".into());
                                }
                                let mut cid_arr = [0u8; 32];
                                cid_arr.copy_from_slice(&cid_bytes);
                                let cid = crate::ContractId::from_bytes(&cid_arr);
                                let payload = if payload_hex.is_empty() {
                                    vec![]
                                } else {
                                    hex::decode(payload_hex)
                                        .map_err(|e| format!("Invalid payload hex: {}", e))?
                                };
                                let result = runtime.query_contract(&cid, method, &payload)?;
                                Ok(serde_json::json!({"result_hex": hex::encode(&result)}))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (400, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Get
                        && (request_url_str.starts_with("/tx/")
                            || request_url_str.starts_with("/api/v1/transactions/"))
                    {
                        let parsed = if let Some(rest) = request_url_str.strip_prefix("/tx/") {
                            if let Some(hash_hex) = rest.strip_suffix("/finality") {
                                Some((hash_hex, true))
                            } else {
                                Some((rest, false))
                            }
                        } else if let Some(rest) =
                            request_url_str.strip_prefix("/api/v1/transactions/")
                        {
                            if let Some(hash_hex) = rest.strip_suffix("/finality") {
                                Some((hash_hex, true))
                            } else {
                                Some((rest, false))
                            }
                        } else {
                            None
                        };

                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let Some((hash_hex, finality_only)) = parsed else {
                                    return Err("Invalid transaction route".into());
                                };
                                if hash_hex.contains('/') || hash_hex.is_empty() {
                                    return Err("Invalid transaction hash path".into());
                                }
                                let hash_bytes = hex::decode(hash_hex)
                                    .map_err(|_| "Invalid transaction hash hex".to_string())?;
                                if hash_bytes.len() != 32 {
                                    return Err("Transaction hash must be 32 bytes".into());
                                }
                                let mut hash_arr = [0u8; 32];
                                hash_arr.copy_from_slice(&hash_bytes);

                                if finality_only {
                                    let finality = runtime
                                        .get_transaction_finality(&hash_arr)?
                                        .ok_or("Transaction not found")?;
                                    return Ok(serde_json::json!({
                                        "hash": hash_hex,
                                        "finality": finality
                                    }));
                                }

                                let mut tx = runtime.get_transaction(&hash_arr)?;
                                if tx.is_none() {
                                    tx = runtime
                                        .get_pending_transactions()?
                                        .into_iter()
                                        .find(|candidate| candidate.hash == hash_arr);
                                }
                                let tx = tx.ok_or("Transaction not found")?;
                                let status = runtime
                                    .get_transaction_status(&hash_arr)?
                                    .unwrap_or(crate::types::TransactionStatus::Pending);
                                let finality = runtime.get_transaction_finality(&hash_arr)?;

                                Ok(serde_json::json!({
                                    "hash": hash_hex,
                                    "status": status,
                                    "finality": finality,
                                    "transaction": tx
                                }))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (404, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Get
                        && matches!(request_url_str, "/block/latest" | "/api/v1/blocks/latest")
                    {
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let chain = runtime.get_chain_state()?;
                                let block = runtime
                                    .get_block_by_height(chain.latest_block_index)?
                                    .ok_or("Latest block not found")?;
                                Ok(serde_json::json!({
                                    "height": block.index,
                                    "hash": hex::encode(block.hash),
                                    "timestamp": block.timestamp,
                                    "tx_count": block.transactions.len(),
                                }))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (404, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Get
                        && (request_url_str.starts_with("/block/by_height/")
                            || (request_url_str.starts_with("/api/v1/blocks/")
                                && request_url_str != "/api/v1/blocks/latest"
                                && !request_url_str.starts_with("/api/v1/blocks/hash/")))
                    {
                        let height_str = if let Some(stripped) =
                            request_url_str.strip_prefix("/block/by_height/")
                        {
                            stripped
                        } else {
                            request_url_str.strip_prefix("/api/v1/blocks/").unwrap_or_default()
                        };
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let height: u64 = height_str
                                    .parse()
                                    .map_err(|_| format!("Invalid height: {}", height_str))?;
                                let block = runtime
                                    .get_block_by_height(height)?
                                    .ok_or(format!("Block not found at height {}", height))?;
                                Ok(crate::cli::block_to_json(&block))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (404, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Get
                        && (request_url_str.starts_with("/block/by_hash/")
                            || request_url_str.starts_with("/api/v1/blocks/hash/"))
                    {
                        let hash_hex = if let Some(stripped) =
                            request_url_str.strip_prefix("/block/by_hash/")
                        {
                            stripped
                        } else {
                            request_url_str.strip_prefix("/api/v1/blocks/hash/").unwrap_or_default()
                        };
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let hash_bytes = hex::decode(hash_hex)
                                    .map_err(|_| "Invalid block hash hex".to_string())?;
                                if hash_bytes.len() != 32 {
                                    return Err("Block hash must be 32 bytes".into());
                                }
                                let mut hash_arr = [0u8; 32];
                                hash_arr.copy_from_slice(&hash_bytes);
                                let block = runtime
                                    .get_block(&hash_arr)?
                                    .ok_or(format!("Block not found for hash {}", hash_hex))?;
                                Ok(crate::cli::block_to_json(&block))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (404, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else if request.method() == &Method::Get
                        && request_url_str.starts_with("/api/v1/accounts/")
                    {
                        let addr_hex = &request_url_str["/api/v1/accounts/".len()..];
                        let response_json =
                            (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                                let pk = parse_pubkey(addr_hex)?;
                                let account =
                                    runtime.get_account(&pk)?.ok_or("Account not found")?;
                                Ok(serde_json::json!({
                                    "address": hex::encode(pk.to_bytes()),
                                    "account": account
                                }))
                            })();
                        let (status, body) = match response_json {
                            Ok(json) => (200, json.to_string()),
                            Err(e) => {
                                (404, serde_json::json!({"error": e.to_string()}).to_string())
                            }
                        };
                        respond_json(request, status, body);
                    } else {
                        let _ = request.respond(
                            Response::from_string("Not Found").with_status_code(StatusCode(404)),
                        );
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    error!("Health endpoint receive error: {}", e);
                    break;
                }
            }
        }
    });

    Ok(())
}

pub fn handle_node(
    action: NodeCommands,
    json: bool,
    backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        NodeCommands::Start {
            config,
            data_dir,
            port,
            daemon,
            foreground_internal,
            peer,
            listen,
            mdns: _mdns,
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
                    .arg("--storage-backend")
                    .arg(backend)
                    .arg("--listen")
                    .arg(&listen)
                    .arg("--foreground-internal");
                if let Some(cfg) = &config {
                    cmd.arg("--config").arg(cfg);
                }
                for p in &peer {
                    cmd.arg("--peer").arg(p);
                }
                if _mdns {
                    cmd.arg("--mdns");
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
            let cfg = Config::load(config.as_deref())
                .map_err(|e| format!("Failed to load config: {}", e))?;
            if !cfg.network.tls_enabled && (!peer.is_empty() || _mdns) {
                log::warn!(
                    "Plain (non-TLS) P2P networking is insecure. \
                     Configure network.tls_enabled=true and provide TLS certs \
                     for production deployments."
                );
            }
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
                let _ = std::fs::remove_file(&pid_path);
            }
            if stop_path.exists() {
                std::fs::remove_file(&stop_path)?;
            }
            let started_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            write_pid_info(&pid_path, std::process::id(), started_at)?;

            #[allow(unused_variables)]
            let (runtime, node_public_key) = build_runtime(&data_dir, &cfg, &peer, &listen, _mdns)?;

            #[cfg(not(feature = "mdns"))]
            if _mdns {
                log::warn!("--mdns flag was used but the 'mdns' cargo feature is not enabled; mDNS discovery will not work");
            }

            #[cfg(feature = "mdns")]
            if _mdns {
                use crate::sync::discovery::MdnsDiscovery;
                let p2p_port =
                    listen.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(9070);
                let discovery = MdnsDiscovery::new(node_public_key, p2p_port)
                    .map_err(|e| format!("mDNS discovery init: {}", e))?;
                discovery.start_announcing().map_err(|e| format!("mDNS announce: {}", e))?;
                if let Ok(discovered) = discovery.browse() {
                    for (addr_str, peer_port) in discovered {
                        let peer_addr = format!("{}:{}", addr_str, peer_port);
                        let _ = runtime.add_peer(&peer_addr);
                    }
                }
                let runtime_for_mdns = runtime.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    if let Ok(discovered) = discovery.browse() {
                        for (addr_str, peer_port) in discovered {
                            let peer_addr = format!("{}:{}", addr_str, peer_port);
                            let _ = runtime_for_mdns.add_peer(&peer_addr);
                        }
                    }
                });
            }

            let health_bind = format!("127.0.0.1:{}", cfg.node.health_port);
            spawn_health_server(runtime.clone(), health_bind)?;
            info!("Node started. Press Ctrl+C to stop.");
            let pc_file = data_dir.join("peer_count");
            let mut heartbeat = 0u64;
            loop {
                if heartbeat.is_multiple_of(10) {
                    let _ = std::fs::write(&pc_file, runtime.sync_layer().peer_count().to_string());
                }
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

                if heartbeat.is_multiple_of(10) {
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
                format!("{}", SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()),
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
            let running = pid_info.as_ref().map(|p| is_pid_running(p.pid)).unwrap_or(false);

            let default_chain = crate::ChainState {
                latest_block_hash: [0u8; 32],
                latest_block_index: 0,
                accounts_root_hash: [0u8; 32],
                total_supply: 0,
            };
            let storage_res: Result<AnyStorage, Box<dyn std::error::Error>> = match backend {
                "redb" => RedbStorage::new(&data_dir).map(AnyStorage::Redb).map_err(|e| e.into()),
                _ => SledStorage::new(&data_dir).map(AnyStorage::Sled).map_err(|e| e.into()),
            };

            let (chain, mempool, storage_locked) = match storage_res {
                Ok(storage) => {
                    let chain =
                        storage.get_chain_state().ok().flatten().unwrap_or(default_chain.clone());
                    let mempool =
                        storage.get_pending_transactions().map(|txs| txs.len()).unwrap_or(0);
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
                peer_count: std::fs::read_to_string(data_dir.join("peer_count"))
                    .ok()
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(0),
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
                    if storage_locked { " | Storage: locked" } else { "" }
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
        NodeCommands::Backup { data_dir, output } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            storage.backup_to(&output)?;
            Ok(text_or_json(
                json,
                &format!("Backup saved to {:?}", output),
                serde_json::json!({"status": "backup_complete", "output": output.to_string_lossy()}),
            ))
        }
        NodeCommands::Restore { data_dir, input } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            storage.restore_from(&input)?;
            Ok(text_or_json(
                json,
                &format!("Restored from {:?}", input),
                serde_json::json!({"status": "restore_complete", "input": input.to_string_lossy()}),
            ))
        }
    }
}

fn handle_config(action: ConfigCommands, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    use crate::config::{generate_default_config, Config};
    match action {
        ConfigCommands::Init { output } => {
            let _cfg = generate_default_config(&output)?;
            Ok(text_or_json(
                json,
                &format!("Config written to {:?}", output),
                serde_json::json!({"status": "ok", "path": output.to_string_lossy()}),
            ))
        }
        ConfigCommands::Set { key, value } => {
            let config_path = std::path::PathBuf::from("config.toml");
            let mut cfg = if config_path.exists() {
                Config::from_file(&config_path)?
            } else {
                Config::default()
            };
            cfg.set(&key, &value)?;
            cfg.save(&config_path)?;
            Ok(text_or_json(
                json,
                &format!("Set {} = {}", key, value),
                serde_json::json!({"status": "ok", "key": key, "value": value}),
            ))
        }
    }
}
