use crate::types::FeeMode;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Component;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("Invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_node_port")]
    pub port: u16,
    #[serde(default = "default_health_port")]
    pub health_port: u16,
    #[serde(default = "default_mempool_limit")]
    pub mempool_limit: usize,
    #[serde(default = "default_ws_port")]
    pub ws_port: u16,
}

fn default_ws_port() -> u16 {
    8081
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    #[serde(default = "default_block_time_ms")]
    pub block_time_ms: u64,
    #[serde(default = "default_max_tx_per_block")]
    pub max_transactions_per_block: u64,
    #[serde(default = "default_authority_key")]
    pub authority_key: String,
    #[serde(default = "default_chain_id")]
    pub chain_id: u64,
    #[serde(default = "default_min_gas_price")]
    pub min_gas_price: u64,
    #[serde(default = "default_finality_depth")]
    pub finality_depth: u64,
    #[serde(default = "default_max_reorg_depth")]
    pub max_reorg_depth: u64,
    #[serde(default = "default_quorum_threshold")]
    pub quorum_threshold: usize,
    #[serde(default = "default_round_robin")]
    pub round_robin: bool,
    #[serde(default = "default_produce_empty_blocks")]
    pub produce_empty_blocks: bool,
    #[serde(default)]
    pub genesis_alloc: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_cache_mb")]
    pub cache_size_mb: u64,
    #[serde(default = "default_compression")]
    pub compression: bool,
    #[serde(default)]
    pub backend: StorageBackend,
    #[serde(default)]
    pub backup_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum StorageBackend {
    #[default]
    Sled,
    Redb,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_max_peers")]
    pub max_peers: u32,
    #[serde(default = "default_conn_timeout_ms")]
    pub connection_timeout_ms: u64,
    #[serde(default)]
    pub tls_enabled: bool,
    #[serde(default)]
    pub tls_cert_path: String,
    #[serde(default)]
    pub tls_key_path: String,
    #[serde(default)]
    pub tls_ca_cert_path: String,
    #[serde(default)]
    pub allowed_peers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_file")]
    pub file: String,
    #[serde(default = "default_json_format")]
    pub json_format: bool,
    #[serde(default = "default_log_max_size_mb")]
    pub log_max_size_mb: u64,
    #[serde(default = "default_log_max_files")]
    pub log_max_files: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeConfig {
    #[serde(default = "default_fee_mode")]
    pub mode: FeeMode,
    #[serde(default)]
    pub base_fee: u64,
    #[serde(default = "default_operator_percent")]
    pub operator_percent: u8,
    #[serde(default = "default_treasury_percent")]
    pub treasury_percent: u8,
    #[serde(default = "default_burn_percent")]
    pub burn_percent: u8,
    #[serde(default)]
    pub treasury_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OracleConfig {
    #[serde(default)]
    pub enabled: bool,
    /// JSON-RPC endpoint for the EVM hub chain (e.g. Arbitrum Sepolia)
    #[serde(default)]
    pub evm_rpc: String,
    /// RewardDistributor contract address on the hub chain
    #[serde(default)]
    pub reward_distributor: String,
    /// LegacyClaimRegistry contract address on the hub chain
    #[serde(default)]
    pub legacy_claim_registry: String,
    /// Amoy (Polygon) JSON-RPC for spoke-chain verification
    #[serde(default)]
    pub amoy_rpc: String,
    /// Env var name that holds the BaaLS EVM signing key (secp256k1, hex)
    #[serde(default = "default_evm_key_env")]
    pub evm_private_key_env: String,
    /// EVM chain ID of the hub (421614 = Arbitrum Sepolia)
    #[serde(default = "default_evm_chain_id")]
    pub evm_chain_id: u64,
}

fn default_evm_key_env() -> String {
    "BAALS_EVM_PRIVATE_KEY".to_string()
}

fn default_evm_chain_id() -> u64 {
    421614
}

// ── Phase L: BaaLS relay bridge watcher config ────────────────────────────────

fn default_log_chunk_size() -> u64 {
    100
}

/// A single spoke chain to watch for BridgeClaimRequested events.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelaySpoke {
    /// Human-readable label used as state-file key (e.g. "amoy", "base-sepolia").
    pub label: String,
    /// JSON-RPC endpoint for this spoke chain.
    pub rpc: String,
    /// CrossChainSender contract address on this spoke chain (0x-prefixed).
    pub sender_address: String,
    /// Block number to begin scanning on the very first run (0 = chain genesis).
    #[serde(default)]
    pub start_block: u64,
    /// Max block range per eth_getLogs call (default 100; reduce if RPC rejects).
    #[serde(default = "default_log_chunk_size")]
    pub log_chunk_size: u64,
}

fn default_relay_poll_secs() -> u64 {
    30
}

fn default_relay_state_path() -> String {
    "/var/lib/baals/relay_state.json".to_string()
}

/// Configuration for the BaaLS relay bridge watcher (Phase L).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelayConfig {
    /// Set to true to enable the relay watcher.
    #[serde(default)]
    pub enabled: bool,
    /// Spoke chains to watch.
    #[serde(default)]
    pub spokes: Vec<RelaySpoke>,
    /// How often to poll spoke chains (seconds, min 10).
    #[serde(default = "default_relay_poll_secs")]
    pub poll_interval_secs: u64,
    /// Path for persisting scanned-block cursors and processed-nonce set.
    #[serde(default = "default_relay_state_path")]
    pub state_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub node: NodeConfig,
    pub consensus: ConsensusConfig,
    pub storage: StorageConfig,
    pub network: NetworkConfig,
    pub logging: LoggingConfig,
    #[serde(default)]
    pub fees: FeeConfig,
    #[serde(default)]
    pub oracle: OracleConfig,
    #[serde(default)]
    pub relay: RelayConfig,
}

fn default_data_dir() -> String {
    "./data".to_string()
}
fn default_node_port() -> u16 {
    8080
}
fn default_health_port() -> u16 {
    8080
}
fn default_mempool_limit() -> usize {
    10000
}
fn default_block_time_ms() -> u64 {
    5000
}
fn default_max_tx_per_block() -> u64 {
    1000
}
fn default_authority_key() -> String {
    String::new()
}
fn default_chain_id() -> u64 {
    1
}
fn default_min_gas_price() -> u64 {
    1
}
fn default_finality_depth() -> u64 {
    12
}
fn default_max_reorg_depth() -> u64 {
    50
}
fn default_quorum_threshold() -> usize {
    1
}
fn default_round_robin() -> bool {
    false
}
fn default_produce_empty_blocks() -> bool {
    false
}
fn default_cache_mb() -> u64 {
    256
}
fn default_compression() -> bool {
    true
}
fn default_max_peers() -> u32 {
    50
}
fn default_conn_timeout_ms() -> u64 {
    30000
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_file() -> String {
    "baals.log".to_string()
}
fn default_json_format() -> bool {
    false
}
fn default_log_max_size_mb() -> u64 {
    100
}
fn default_log_max_files() -> u32 {
    5
}

fn default_fee_mode() -> FeeMode {
    FeeMode::Economic
}

fn default_operator_percent() -> u8 {
    70
}

fn default_treasury_percent() -> u8 {
    20
}

fn default_burn_percent() -> u8 {
    10
}

impl Default for FeeConfig {
    fn default() -> Self {
        Self {
            mode: default_fee_mode(),
            base_fee: 0,
            operator_percent: default_operator_percent(),
            treasury_percent: default_treasury_percent(),
            burn_percent: default_burn_percent(),
            treasury_address: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            node: NodeConfig {
                data_dir: default_data_dir(),
                port: default_node_port(),
                health_port: default_health_port(),
                mempool_limit: default_mempool_limit(),
                ws_port: default_ws_port(),
            },
            consensus: ConsensusConfig {
                block_time_ms: default_block_time_ms(),
                max_transactions_per_block: default_max_tx_per_block(),
                authority_key: default_authority_key(),
                chain_id: default_chain_id(),
                min_gas_price: default_min_gas_price(),
                finality_depth: default_finality_depth(),
                max_reorg_depth: default_max_reorg_depth(),
                quorum_threshold: default_quorum_threshold(),
                round_robin: default_round_robin(),
                produce_empty_blocks: default_produce_empty_blocks(),
                genesis_alloc: std::collections::HashMap::new(),
            },
            storage: StorageConfig {
                cache_size_mb: default_cache_mb(),
                compression: default_compression(),
                backend: StorageBackend::default(),
                backup_interval_secs: 0,
            },
            network: NetworkConfig {
                max_peers: default_max_peers(),
                connection_timeout_ms: default_conn_timeout_ms(),
                tls_enabled: false,
                tls_cert_path: String::new(),
                tls_key_path: String::new(),
                tls_ca_cert_path: String::new(),
                allowed_peers: Vec::new(),
            },
            logging: LoggingConfig {
                level: default_log_level(),
                file: default_log_file(),
                json_format: default_json_format(),
                log_max_size_mb: default_log_max_size_mb(),
                log_max_files: default_log_max_files(),
            },
            fees: FeeConfig::default(),
            oracle: OracleConfig::default(),
            relay: RelayConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        // Environment variable overrides take priority
        if let Ok(env_path) = std::env::var("BAALS_CONFIG") {
            return Self::from_file(Path::new(&env_path));
        }

        // Check provided path, then default paths
        if let Some(p) = path {
            if p.exists() {
                return Self::from_file(p);
            }
        }

        let default_paths = [Path::new("config.toml"), Path::new("baals.toml")];

        for p in &default_paths {
            if p.exists() {
                return Self::from_file(p);
            }
        }

        Ok(Config::default())
    }

    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.node.data_dir.is_empty() {
            return Err(ConfigError::Invalid("data_dir cannot be empty".to_string()));
        }
        if self.consensus.block_time_ms < 100 {
            return Err(ConfigError::Invalid("block_time_ms must be >= 100".to_string()));
        }
        if self.consensus.finality_depth == 0 {
            return Err(ConfigError::Invalid("finality_depth must be >= 1".to_string()));
        }
        if self.consensus.max_reorg_depth == 0 {
            return Err(ConfigError::Invalid("max_reorg_depth must be >= 1".to_string()));
        }
        if !["trace", "debug", "info", "warn", "error"].contains(&self.logging.level.as_str()) {
            return Err(ConfigError::Invalid(format!("Invalid log level: {}", self.logging.level)));
        }
        for pk_hex in self.consensus.genesis_alloc.keys() {
            let decoded = hex::decode(pk_hex).map_err(|_| {
                ConfigError::Invalid(format!("Invalid genesis_alloc key hex: {}", pk_hex))
            })?;
            if decoded.len() != 32 {
                return Err(ConfigError::Invalid(format!(
                    "genesis_alloc key must be 32 bytes (64 hex chars), got {} bytes: {}",
                    decoded.len(),
                    pk_hex
                )));
            }
        }
        let fee_split_total = self
            .fees
            .operator_percent
            .saturating_add(self.fees.treasury_percent)
            .saturating_add(self.fees.burn_percent);
        if fee_split_total != 100 {
            return Err(ConfigError::Invalid(format!(
                "Invalid fee split: operator({}) + treasury({}) + burn({}) must equal 100",
                self.fees.operator_percent, self.fees.treasury_percent, self.fees.burn_percent
            )));
        }
        if let Some(addr) = self.fees.treasury_address.as_ref() {
            let trimmed = addr.trim();
            if !trimmed.is_empty() {
                let decoded = hex::decode(trimmed).map_err(|_| {
                    ConfigError::Invalid("Invalid fees.treasury_address hex".into())
                })?;
                if decoded.len() != 32 {
                    return Err(ConfigError::Invalid(
                        "fees.treasury_address must be 32 bytes (64 hex chars)".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        match key {
            "node.data_dir" => self.node.data_dir = value.to_string(),
            "node.port" => {
                self.node.port =
                    value.parse().map_err(|_| ConfigError::Invalid("Invalid port".into()))?
            }
            "node.health_port" => {
                self.node.health_port =
                    value.parse().map_err(|_| ConfigError::Invalid("Invalid health_port".into()))?
            }
            "node.ws_port" => {
                self.node.ws_port =
                    value.parse().map_err(|_| ConfigError::Invalid("Invalid ws_port".into()))?
            }
            "node.mempool_limit" => {
                self.node.mempool_limit = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid mempool_limit".into()))?
            }
            "consensus.block_time_ms" => {
                self.consensus.block_time_ms = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid block_time_ms".into()))?
            }
            "consensus.max_transactions_per_block" => {
                self.consensus.max_transactions_per_block = value.parse().map_err(|_| {
                    ConfigError::Invalid("Invalid max_transactions_per_block".into())
                })?
            }
            "consensus.authority_key" => self.consensus.authority_key = value.to_string(),
            "consensus.chain_id" => {
                self.consensus.chain_id =
                    value.parse().map_err(|_| ConfigError::Invalid("Invalid chain_id".into()))?
            }
            "consensus.min_gas_price" => {
                self.consensus.min_gas_price = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid min_gas_price".into()))?
            }
            "consensus.finality_depth" => {
                self.consensus.finality_depth = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid finality_depth".into()))?
            }
            "consensus.max_reorg_depth" => {
                self.consensus.max_reorg_depth = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid max_reorg_depth".into()))?
            }
            "consensus.quorum_threshold" => {
                self.consensus.quorum_threshold = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid quorum_threshold".into()))?
            }
            "consensus.round_robin" => {
                self.consensus.round_robin = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid round_robin (true/false)".into()))?
            }
            "consensus.produce_empty_blocks" => {
                self.consensus.produce_empty_blocks = value.parse().map_err(|_| {
                    ConfigError::Invalid("Invalid produce_empty_blocks (true/false)".into())
                })?
            }
            "network.max_peers" => {
                self.network.max_peers =
                    value.parse().map_err(|_| ConfigError::Invalid("Invalid max_peers".into()))?
            }
            "network.connection_timeout_ms" => {
                self.network.connection_timeout_ms = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid connection_timeout_ms".into()))?
            }
            "logging.level" => {
                self.logging.level = value.to_string();
                self.validate()?;
            }
            "logging.file" => self.logging.file = value.to_string(),
            "logging.json_format" => {
                self.logging.json_format = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid json_format (true/false)".into()))?
            }
            "logging.log_max_size_mb" => {
                self.logging.log_max_size_mb = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid log_max_size_mb".into()))?
            }
            "logging.log_max_files" => {
                self.logging.log_max_files = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid log_max_files".into()))?
            }
            "storage.cache_size_mb" => {
                self.storage.cache_size_mb = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid cache_size_mb".into()))?
            }
            "storage.compression" => {
                self.storage.compression = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid compression (true/false)".into()))?
            }
            "storage.backend" => {
                self.storage.backend = match value.to_lowercase().as_str() {
                    "sled" => StorageBackend::Sled,
                    "redb" => StorageBackend::Redb,
                    _ => return Err(ConfigError::Invalid(format!("Unknown backend: {}", value))),
                }
            }
            "storage.backup_interval_secs" => {
                self.storage.backup_interval_secs = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid backup_interval_secs".into()))?
            }
            "network.tls_enabled" => {
                self.network.tls_enabled = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid tls_enabled (true/false)".into()))?
            }
            "network.tls_cert_path" => self.network.tls_cert_path = value.to_string(),
            "network.tls_key_path" => self.network.tls_key_path = value.to_string(),
            "network.tls_ca_cert_path" => self.network.tls_ca_cert_path = value.to_string(),
            "network.allowed_peers" => {
                self.network.allowed_peers = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .collect();
            }
            "fees.mode" => {
                self.fees.mode = match value.to_ascii_lowercase().as_str() {
                    "none" => FeeMode::None,
                    "metered" => FeeMode::Metered,
                    "economic" => FeeMode::Economic,
                    _ => {
                        return Err(ConfigError::Invalid(
                            "Invalid fees.mode (expected: none|metered|economic)".into(),
                        ))
                    }
                }
            }
            "fees.base_fee" => {
                self.fees.base_fee = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid fees.base_fee".into()))?
            }
            "fees.operator_percent" => {
                self.fees.operator_percent = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid fees.operator_percent".into()))?
            }
            "fees.treasury_percent" => {
                self.fees.treasury_percent = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid fees.treasury_percent".into()))?
            }
            "fees.burn_percent" => {
                self.fees.burn_percent = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("Invalid fees.burn_percent".into()))?
            }
            "fees.treasury_address" => {
                let trimmed = value.trim();
                self.fees.treasury_address =
                    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
            }
            _ => return Err(ConfigError::Invalid(format!("Unknown config key: {}", key))),
        }
        self.validate()
    }
}

/// Checks that the given path does not escape the data directory via `..` components.
fn is_safe_path(path: &Path) -> bool {
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return false;
        }
    }
    true
}

/// Initialize logging based on configuration. Uses `env_logger` with optional JSON output.
///
/// If `config.logging.json_format` is true, each log line is emitted as:
/// `{"timestamp":"...","level":"...","module":"...","message":"..."}`
///
/// If `config.logging.log_max_size_mb > 0`, a rotating file logger writes to
/// `config.logging.file`, rotating files when the size limit is reached.
///
/// If a logger is already initialized, this call is a no-op.
pub fn setup_logging(config: &Config, default_level: &str) -> Result<(), ConfigError> {
    // CLI-provided level (from --verbose) takes precedence over config file.
    // Config file can override only if the CLI didn't set a non-default level.
    let effective_level = if default_level != "info" {
        default_level.to_string()
    } else {
        config.logging.level.clone()
    };
    let level_filter = match effective_level.as_str() {
        "trace" => log::LevelFilter::Trace,
        "debug" => log::LevelFilter::Debug,
        "info" => log::LevelFilter::Info,
        "warn" => log::LevelFilter::Warn,
        "error" => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };

    let mut builder = env_logger::Builder::new();
    builder.filter_level(level_filter);

    if config.logging.json_format {
        builder.format(|buf, record| {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            writeln!(
                buf,
                r#"{{"timestamp":"{:.3}","level":"{}","module":"{}","message":"{}"}}"#,
                timestamp,
                record.level(),
                record.module_path().unwrap_or("unknown"),
                record.args()
            )
        });
    } else {
        builder.format_timestamp_secs();
    }

    if config.logging.log_max_size_mb > 0 && !config.logging.file.is_empty() {
        let log_path = PathBuf::from(&config.logging.file);
        if !is_safe_path(&log_path) {
            return Err(ConfigError::Invalid(
                "Log file path must not contain '..' components (path traversal)".to_string(),
            ));
        }
        let max_size = config.logging.log_max_size_mb * 1024 * 1024;
        let max_files = config.logging.log_max_files;
        let writer = RotatingFileWriter::new(log_path, max_size, max_files)
            .map_err(|e| ConfigError::Invalid(format!("Failed to open log file: {}", e)))?;
        builder.target(env_logger::Target::Pipe(Box::new(TeeWriter { file: writer })));
    }

    let _ = builder.try_init();
    Ok(())
}

struct RotatingFileWriter {
    file: std::fs::File,
    path: PathBuf,
    max_size: u64,
    max_files: u32,
}

impl RotatingFileWriter {
    fn new(path: PathBuf, max_size: u64, max_files: u32) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { file, path, max_size, max_files })
    }

    fn maybe_rotate(&mut self) -> std::io::Result<()> {
        if self.max_size == 0 {
            return Ok(());
        }
        let current_size = self.file.metadata()?.len();
        if current_size >= self.max_size {
            for i in (1..self.max_files).rev() {
                let from = self.path.with_extension(format!("{}.log", i));
                let to = self.path.with_extension(format!("{}.log", i + 1));
                if from.exists() {
                    let _ = std::fs::rename(&from, &to);
                }
            }
            let backup = self.path.with_extension("1.log");
            let _ = std::fs::rename(&self.path, &backup);
            self.file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        }
        Ok(())
    }
}

impl std::io::Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(buf)?;
        let _ = self.file.flush();
        self.maybe_rotate()?;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

struct TeeWriter {
    file: RotatingFileWriter,
}

impl std::io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        self.file.flush()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    pub running: bool,
    pub chain_height: u64,
    pub latest_block_hash: String,
    pub mempool_size: usize,
    pub peer_count: usize,
    pub uptime_seconds: u64,
}

/// Generate a default config file and write it to disk
pub fn generate_default_config(path: &Path) -> Result<Config, ConfigError> {
    let config = Config::default();
    config.save(path)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn set_supports_consensus_chain_id_and_related_keys() {
        let mut cfg = Config::default();

        cfg.set("consensus.chain_id", "42").expect("set chain_id");
        cfg.set("consensus.min_gas_price", "7").expect("set min_gas_price");
        cfg.set("consensus.quorum_threshold", "2").expect("set quorum_threshold");
        cfg.set("consensus.round_robin", "true").expect("set round_robin");
        cfg.set("consensus.produce_empty_blocks", "true").expect("set produce_empty_blocks");

        assert_eq!(cfg.consensus.chain_id, 42);
        assert_eq!(cfg.consensus.min_gas_price, 7);
        assert_eq!(cfg.consensus.quorum_threshold, 2);
        assert_eq!(cfg.consensus.round_robin, true);
        assert_eq!(cfg.consensus.produce_empty_blocks, true);
    }

    #[test]
    fn set_supports_ws_port_and_allowed_peers() {
        let mut cfg = Config::default();

        cfg.set("node.ws_port", "9191").expect("set ws_port");
        cfg.set("network.allowed_peers", "1.2.3.4:9070, 5.6.7.8:9070").expect("set allowed_peers");

        assert_eq!(cfg.node.ws_port, 9191);
        assert_eq!(
            cfg.network.allowed_peers,
            vec!["1.2.3.4:9070".to_string(), "5.6.7.8:9070".to_string()]
        );
    }

    #[test]
    fn test_genesis_alloc_validation() {
        let mut cfg = Config::default();
        // Valid 32-byte hex public key
        let valid_pk = "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc";
        cfg.consensus.genesis_alloc.insert(valid_pk.to_string(), 1000);
        assert!(cfg.validate().is_ok());

        // Invalid hex character
        let invalid_hex = "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abg";
        let mut cfg2 = Config::default();
        cfg2.consensus.genesis_alloc.insert(invalid_hex.to_string(), 1000);
        assert!(cfg2.validate().is_err());

        // Wrong key size (31 bytes instead of 32)
        let invalid_size = "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537ab";
        let mut cfg3 = Config::default();
        cfg3.consensus.genesis_alloc.insert(invalid_size.to_string(), 1000);
        assert!(cfg3.validate().is_err());
    }
}
