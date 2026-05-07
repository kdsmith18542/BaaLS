use serde::{Deserialize, Serialize};
use std::io::Write;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    #[serde(default = "default_block_time_ms")]
    pub block_time_ms: u64,
    #[serde(default = "default_max_tx_per_block")]
    pub max_transactions_per_block: u64,
    #[serde(default = "default_authority_key")]
    pub authority_key: String,
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
pub struct Config {
    pub node: NodeConfig,
    pub consensus: ConsensusConfig,
    pub storage: StorageConfig,
    pub network: NetworkConfig,
    pub logging: LoggingConfig,
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

impl Default for Config {
    fn default() -> Self {
        Self {
            node: NodeConfig {
                data_dir: default_data_dir(),
                port: default_node_port(),
                health_port: default_health_port(),
                mempool_limit: default_mempool_limit(),
            },
            consensus: ConsensusConfig {
                block_time_ms: default_block_time_ms(),
                max_transactions_per_block: default_max_tx_per_block(),
                authority_key: default_authority_key(),
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
            },
            logging: LoggingConfig {
                level: default_log_level(),
                file: default_log_file(),
                json_format: default_json_format(),
                log_max_size_mb: default_log_max_size_mb(),
                log_max_files: default_log_max_files(),
            },
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
        if !["trace", "debug", "info", "warn", "error"].contains(&self.logging.level.as_str()) {
            return Err(ConfigError::Invalid(format!("Invalid log level: {}", self.logging.level)));
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
            _ => return Err(ConfigError::Invalid(format!("Unknown config key: {}", key))),
        }
        Ok(())
    }
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
pub fn setup_logging(config: &Config, _default_level: &str) -> Result<(), ConfigError> {
    let level_filter = match config.logging.level.as_str() {
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
        let max_size = config.logging.log_max_size_mb * 1024 * 1024;
        let max_files = config.logging.log_max_files;
        let writer = RotatingFileWriter::new(log_path, max_size, max_files)
            .map_err(|e| ConfigError::Invalid(format!("Failed to open log file: {}", e)))?;
        builder.target(env_logger::Target::Pipe(Box::new(writer)));
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

pub fn format_for_logging(config: &LoggingConfig) -> LogFormat {
    if config.json_format {
        LogFormat::Json
    } else {
        LogFormat::Text
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
