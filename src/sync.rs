use async_trait::async_trait;
use bincode;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};
use hex;
use log;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::RwLock;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

use crate::storage::Storage;
use crate::types::{Block, ChainState, PublicKey};

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Synchronization error: {0}")]
    SynchronizationError(String),
    #[error("Block not found during sync")]
    BlockNotFound,
    #[error("Connection timeout")]
    ConnectionTimeout,
    #[error("Peer authentication failed")]
    AuthenticationFailed,
    #[error("Invalid message format")]
    InvalidMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub id: PublicKey,
    pub address: SocketAddr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkMessage {
    // Handshake messages
    Handshake {
        peer_id: PublicKey,
        version: u32,
        challenge: [u8; 32],
        listen_port: u16,
        chain_id: u64,
    },
    HandshakeAck {
        peer_id: PublicKey,
        version: u32,
        signature: Vec<u8>,
        challenge: [u8; 32],
        listen_port: u16,
        chain_id: u64,
    },
    HandshakeVerify {
        signature: Vec<u8>,
    },

    // Sync protocol messages
    GetChainHead,
    ChainHeadResponse {
        latest_block_hash: [u8; 32],
        height: u64,
    },
    GetBlocks {
        from_height: u64,
        to_height: u64,
    },
    BlocksResponse {
        blocks: Vec<Block>,
    },
    NewBlockAnnouncement {
        block_hash: [u8; 32],
        height: u64,
    },
    QuorumSignatureRequest {
        block: Block,
    },
    QuorumSignatureResponse {
        accepted: bool,
        signer: Option<String>,
        signature: Option<Vec<u8>>,
        reason: Option<String>,
    },

    // Fork resolution
    ForkResolution {
        common_height: u64,
        fork_blocks: Vec<Block>,
    },
    GetForkBlocks {
        from_height: u64,
        to_height: u64,
    },
    ForkBlocksResponse {
        blocks: Vec<Block>,
        total_height: u64,
    },

    // Keep-alive
    Ping,
    Pong,
    PeerList {
        peers: Vec<(PublicKey, String)>,
    },
    RequestBlock {
        hash: [u8; 32],
    },
    BlockResponse {
        block: Option<crate::types::Block>,
    },
    GetSnapshot {
        height: u64,
    },
    SnapshotResponse {
        height: u64,
        snapshot: Option<Vec<u8>>,
        block: Option<crate::types::Block>,
    },
}

#[derive(Debug)]
pub struct MessageFrame {
    pub length: u32,
    pub message: NetworkMessage,
}

impl MessageFrame {
    pub fn new(message: NetworkMessage) -> Result<Self, SyncError> {
        let message_bytes = bincode::serialize(&message)
            .map_err(|e| SyncError::SerializationError(e.to_string()))?;
        Ok(MessageFrame { length: message_bytes.len() as u32, message })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, SyncError> {
        let message_bytes = bincode::serialize(&self.message)
            .map_err(|e| SyncError::SerializationError(e.to_string()))?;
        let mut frame = Vec::new();
        frame.extend_from_slice(&self.length.to_le_bytes());
        frame.extend_from_slice(&message_bytes);
        Ok(frame)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, SyncError> {
        if data.len() < 4 {
            return Err(SyncError::InvalidMessage);
        }

        let length = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if data.len() < (length + 4) as usize {
            return Err(SyncError::InvalidMessage);
        }

        let message_bytes = &data[4..(length + 4) as usize];
        let message: NetworkMessage = bincode::deserialize(message_bytes)
            .map_err(|e| SyncError::SerializationError(e.to_string()))?;

        Ok(MessageFrame { length, message })
    }
}

#[async_trait]
pub trait SyncLayer: Send + Sync {
    /// Attempts to synchronize the local ledger with a peer.
    async fn sync_with_peer(
        &self,
        peer: &Peer,
        local_chain_state: &ChainState,
    ) -> Result<Block, SyncError>;

    /// Discovers new peers in the network.
    async fn discover_peers(&self) -> Result<Vec<Peer>, SyncError>;

    /// Broadcasts a new block to known peers.
    async fn broadcast_block(&self, block: &Block, peers: &[Peer]) -> Result<(), SyncError>;

    /// Request quorum signatures for a proposed block from known peers.
    async fn request_quorum_signatures(
        &self,
        _block: &Block,
        _peers: &[Peer],
    ) -> Result<Vec<(String, Vec<u8>)>, SyncError> {
        Ok(Vec::new())
    }

    /// Best-effort peer count for status reporting.
    fn peer_count(&self) -> usize {
        0
    }

    /// Start the P2P listener server. Default no-op for NoopSync.
    async fn start_listener(&self) -> Result<(), SyncError> {
        Ok(())
    }

    /// Add a peer by address string (e.g. "127.0.0.1:9000").
    fn add_peer_by_address(&self, _addr: &str) -> Result<(), SyncError> {
        Ok(())
    }

    /// Remove a peer by address.
    fn remove_peer(&self, _addr: &str) -> Result<(), SyncError> {
        Ok(())
    }

    /// Return list of currently known peer addresses.
    fn known_peers(&self) -> Vec<String> {
        vec![]
    }

    /// Trigger an immediate sync cycle.
    fn trigger_sync(&self) -> Result<(), SyncError> {
        Ok(())
    }

    /// Drain and return blocks received from peers since last poll.
    fn poll_received_blocks(&self) -> Vec<Block> {
        vec![]
    }

    /// Stop the P2P listener server. Default no-op for NoopSync.
    fn stop_listener(&self) {}
}

/// Per-peer token-bucket rate limiter to prevent message floods.
struct PerPeerRateLimiter {
    tokens: f64,
    last_refill: std::time::Instant,
}

impl PerPeerRateLimiter {
    fn new() -> Self {
        Self { tokens: MAX_P2P_MESSAGES_PER_SECOND as f64, last_refill: std::time::Instant::now() }
    }

    fn allow(&mut self) -> bool {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * MAX_P2P_MESSAGES_PER_SECOND as f64)
            .min(MAX_P2P_MESSAGES_PER_SECOND as f64);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Per-peer health tracking for connection quality and backoff.
#[derive(Debug, Clone)]
struct PeerHealth {
    consecutive_failures: u32,
    last_failure: Option<std::time::Instant>,
    last_success: Option<std::time::Instant>,
    total_successes: u64,
    total_failures: u64,
}

impl PeerHealth {
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
            last_failure: None,
            last_success: None,
            total_successes: 0,
            total_failures: 0,
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.last_success = Some(std::time::Instant::now());
        self.total_successes += 1;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.last_failure = Some(std::time::Instant::now());
        self.total_failures += 1;
    }

    /// Exponential backoff: 2^failures seconds, capped at 600s (10 min).
    fn backoff_remaining(&self) -> Option<std::time::Duration> {
        let last = self.last_failure?;
        if self.consecutive_failures == 0 {
            return None;
        }
        let backoff_secs = (2u64.saturating_pow(self.consecutive_failures.min(10))).min(600);
        let elapsed = last.elapsed();
        let backoff = std::time::Duration::from_secs(backoff_secs);
        if elapsed < backoff {
            Some(backoff - elapsed)
        } else {
            None
        }
    }

    fn is_dead(&self) -> bool {
        self.consecutive_failures >= PEER_MAX_CONSECUTIVE_FAILURES
    }
}

/// Minimal custom P2P sync implementation
pub struct CustomSync {
    peer_id: PublicKey,
    known_peers: Arc<tokio::sync::RwLock<HashMap<PublicKey, SocketAddr>>>,
    authorized_peers: Arc<tokio::sync::RwLock<Option<HashSet<PublicKey>>>>,
    block_cache: Arc<Mutex<HashMap<[u8; 32], Block>>>,
    listen_addr: SocketAddr,
    is_running: Arc<Mutex<bool>>,
    tls_config: Option<Arc<TlsConfig>>,
    tls_insecure: bool,
    storage: Arc<Mutex<Option<Box<dyn Storage>>>>,
    received_blocks: Arc<std::sync::Mutex<Vec<Block>>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
    connection_semaphore: Arc<tokio::sync::Semaphore>,
    signing_key: Arc<Mutex<Option<SigningKey>>>,
    peer_rate_limiters: Arc<Mutex<HashMap<SocketAddr, PerPeerRateLimiter>>>,
    peer_health: Arc<Mutex<HashMap<SocketAddr, PeerHealth>>>,
    outbound_connections: Arc<Mutex<HashMap<SocketAddr, Box<dyn AsyncStream>>>>,
    chain_id: u64,
    announcement_tx: tokio::sync::broadcast::Sender<NetworkMessage>,
    bootstrap_peers: Arc<std::sync::Mutex<Vec<SocketAddr>>>,
    snapshots_dir: Option<std::path::PathBuf>,
    connection_timeout: Duration,
}

const MAX_CONCURRENT_CONNECTIONS: usize = 128;
const MAX_KNOWN_PEERS: usize = 2048;
const MAX_RECEIVED_BLOCKS_QUEUE: usize = 1000;
const MAX_BLOCK_CACHE_SIZE: usize = 5000;
const MAX_P2P_MESSAGES_PER_SECOND: u32 = 50;
const MAX_BLOCK_RANGE: u64 = 500;
const PEER_MAX_CONSECUTIVE_FAILURES: u32 = 10;

impl std::fmt::Debug for CustomSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomSync")
            .field("peer_id", &self.peer_id)
            .field("listen_addr", &self.listen_addr)
            .field("tls_config", &self.tls_config)
            .finish()
    }
}

/// TLS configuration for P2P connections
pub struct TlsConfig {
    pub server_config: tokio_rustls::rustls::ServerConfig,
    pub client_config: tokio_rustls::rustls::ClientConfig,
    pub cert_pins: Arc<RwLock<HashSet<Vec<u8>>>>,
}

impl std::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConfig").finish_non_exhaustive()
    }
}

impl Clone for TlsConfig {
    fn clone(&self) -> Self {
        TlsConfig {
            server_config: self.server_config.clone(),
            client_config: self.client_config.clone(),
            cert_pins: Arc::clone(&self.cert_pins),
        }
    }
}

impl TlsConfig {
    /// Load TLS config from certificate and key files.
    /// If ca_cert_path is provided, enables server certificate verification
    /// for outbound connections using the CA certificate(s).
    pub fn load(
        cert_path: &str,
        key_path: &str,
        ca_cert_path: Option<&str>,
    ) -> Result<Self, SyncError> {
        use rustls_pemfile::{certs, pkcs8_private_keys};
        use std::io::BufReader;

        // Load server certificate chain
        let cert_file = std::fs::File::open(cert_path)
            .map_err(|e| SyncError::NetworkError(format!("Failed to open cert file: {}", e)))?;
        let mut cert_reader = BufReader::new(cert_file);
        let cert_chain: Vec<rustls::pki_types::CertificateDer> = certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| SyncError::NetworkError(format!("Failed to parse cert: {}", e)))?;

        // Load private key
        let key_file = std::fs::File::open(key_path)
            .map_err(|e| SyncError::NetworkError(format!("Failed to open key file: {}", e)))?;
        let mut key_reader = BufReader::new(key_file);
        let keys: Vec<rustls::pki_types::PrivateKeyDer> = pkcs8_private_keys(&mut key_reader)
            .filter_map(|k| k.ok())
            .map(rustls::pki_types::PrivateKeyDer::Pkcs8)
            .collect();
        if keys.is_empty() {
            return Err(SyncError::NetworkError("No private keys found in key file".to_string()));
        }

        // Build server config
        let server_config = tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain.clone(), keys[0].clone_key())
            .map_err(|e| {
                SyncError::NetworkError(format!("Failed to build server TLS config: {}", e))
            })?;

        // Build client config with certificate pinning
        let cert_pins = Arc::new(RwLock::new(HashSet::new()));

        // If a CA certificate path is provided, load and pin trust certificate fingerprints.
        if let Some(ca_path) = ca_cert_path {
            let ca_file = std::fs::File::open(ca_path).map_err(|e| {
                SyncError::NetworkError(format!("Failed to open CA cert file: {}", e))
            })?;
            let mut ca_reader = BufReader::new(ca_file);
            let ca_certs: Vec<rustls::pki_types::CertificateDer> = certs(&mut ca_reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| SyncError::NetworkError(format!("Failed to parse CA cert: {}", e)))?;
            {
                let mut pins =
                    cert_pins.write().map_err(|e| SyncError::NetworkError(e.to_string()))?;
                for ca_cert in &ca_certs {
                    let fingerprint = Sha256::digest(ca_cert.as_ref()).to_vec();
                    log::info!(
                        "Pinned trust certificate with SHA256 fingerprint: {}",
                        hex::encode(&fingerprint)
                    );
                    pins.insert(fingerprint);
                }
            }
        }

        let client_config = tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(CertificatePinner::new(Arc::clone(
                &cert_pins,
            ))))
            .with_no_client_auth();

        Ok(TlsConfig { server_config, client_config, cert_pins })
    }

    /// Generate a self-signed TLS config for development
    pub fn generate_self_signed() -> Result<Self, SyncError> {
        use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

        let mut params = CertificateParams::new(Vec::new())
            .map_err(|e| SyncError::NetworkError(format!("rcgen error: {}", e)))?;
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, "baals-node");
        params.distinguished_name.push(DnType::OrganizationName, "BaaLS");

        let key_pair = KeyPair::generate()
            .map_err(|e| SyncError::NetworkError(format!("Failed to generate key pair: {}", e)))?;
        let cert = params.self_signed(&key_pair).map_err(|e| {
            SyncError::NetworkError(format!("Failed to generate self-signed cert: {}", e))
        })?;

        let cert_der = cert.der().clone();
        let key_der = key_pair.serialize_der();

        let server_config = tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert_der],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(key_der),
                ),
            )
            .map_err(|e| SyncError::NetworkError(format!("Server TLS config: {}", e)))?;

        let cert_pins = Arc::new(RwLock::new(HashSet::new()));
        let client_config = tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(CertificatePinner::new(Arc::clone(
                &cert_pins,
            ))))
            .with_no_client_auth();

        Ok(TlsConfig { server_config, client_config, cert_pins })
    }

    /// Add a certificate pin from a hex-encoded SHA256 fingerprint.
    /// Once pins are set, only certificates matching a pinned fingerprint are accepted.
    pub fn add_cert_pin(&self, fingerprint_hex: &str) -> Result<(), SyncError> {
        let bytes = hex::decode(fingerprint_hex)
            .map_err(|e| SyncError::NetworkError(format!("Invalid hex fingerprint: {}", e)))?;
        if bytes.len() != 32 {
            return Err(SyncError::NetworkError(
                "Fingerprint must be 32 bytes (SHA256)".to_string(),
            ));
        }
        let mut pins =
            self.cert_pins.write().map_err(|e| SyncError::NetworkError(e.to_string()))?;
        pins.insert(bytes);
        log::info!("Added certificate pin: {} ({} total)", fingerprint_hex, pins.len());
        Ok(())
    }
}

/// Certificate pinning verifier — checks SHA256 fingerprint against pin set.
/// If pin set is empty, accepts any certificate (backward compatible).
struct CertificatePinner {
    cert_pins: Arc<RwLock<HashSet<Vec<u8>>>>,
    provider: rustls::crypto::CryptoProvider,
}

impl std::fmt::Debug for CertificatePinner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificatePinner").finish_non_exhaustive()
    }
}

impl CertificatePinner {
    fn new(cert_pins: Arc<RwLock<HashSet<Vec<u8>>>>) -> Self {
        Self { cert_pins, provider: rustls::crypto::aws_lc_rs::default_provider() }
    }
}

impl rustls::client::danger::ServerCertVerifier for CertificatePinner {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer,
        intermediates: &[rustls::pki_types::CertificateDer],
        _server_name: &rustls::pki_types::ServerName,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let pins = self.cert_pins.read().map_err(|e| rustls::Error::General(e.to_string()))?;
        if pins.is_empty() {
            // No pins configured: accept any valid certificate (development mode).
            // The application-layer Ed25519 challenge-response handshake still
            // provides mutual authentication. Production deployments MUST
            // configure certificate pins via tls_ca_cert_path or add_cert_pin().
            return Ok(rustls::client::danger::ServerCertVerified::assertion());
        }
        let leaf_hash = Sha256::digest(end_entity.as_ref()).to_vec();
        if pins.contains(&leaf_hash) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else if intermediates
            .iter()
            .any(|cert| pins.contains(&Sha256::digest(cert.as_ref()).to_vec()))
        {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "certificate fingerprint not in pin set (leaf or intermediates)".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

impl CustomSync {
    pub fn new(peer_id: PublicKey, listen_addr: SocketAddr, chain_id: u64) -> Self {
        let (announcement_tx, _) = tokio::sync::broadcast::channel(16);
        Self {
            peer_id,
            known_peers: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            authorized_peers: Arc::new(tokio::sync::RwLock::new(None)),
            block_cache: Arc::new(Mutex::new(HashMap::new())),
            listen_addr,
            is_running: Arc::new(Mutex::new(false)),
            tls_config: None,
            tls_insecure: false,
            storage: Arc::new(Mutex::new(None)),
            received_blocks: Arc::new(std::sync::Mutex::new(Vec::new())),
            shutdown_tx: Arc::new(Mutex::new(None)),
            connection_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONNECTIONS)),
            signing_key: Arc::new(Mutex::new(None)),
            peer_rate_limiters: Arc::new(Mutex::new(HashMap::new())),
            peer_health: Arc::new(Mutex::new(HashMap::new())),
            outbound_connections: Arc::new(Mutex::new(HashMap::new())),
            chain_id,
            announcement_tx,
            bootstrap_peers: Arc::new(std::sync::Mutex::new(Vec::new())),
            snapshots_dir: None,
            connection_timeout: Duration::from_secs(10),
        }
    }

    pub fn with_snapshots_dir(mut self, path: std::path::PathBuf) -> Self {
        self.snapshots_dir = Some(path);
        self
    }

    pub fn with_connection_timeout(mut self, timeout: Duration) -> Self {
        // Avoid degenerate values that cause immediate disconnect churn.
        self.connection_timeout = timeout.max(Duration::from_secs(1));
        self
    }

    pub fn with_signing_key(self, key: SigningKey) -> Self {
        *self.signing_key.try_lock().unwrap() = Some(key);
        self
    }

    pub fn with_storage(self, storage: Box<dyn Storage>) -> Self {
        *self.storage.try_lock().expect("storage lock during init") = Some(storage);
        self
    }

    pub fn with_tls_insecure(self) -> Self {
        log::warn!("TLS running in insecure mode: accepting all peer certificates. Set tls_insecure=false in production.");
        Self { tls_insecure: true, ..self }
    }

    pub fn with_tls(mut self, tls_config: TlsConfig) -> Self {
        self.tls_config = Some(Arc::new(tls_config));
        self
    }

    pub fn with_authorized_peers(self, peers: HashSet<PublicKey>) -> Self {
        *self.authorized_peers.blocking_write() = Some(peers);
        self
    }

    async fn is_peer_authorized(&self, peer_id: &PublicKey) -> bool {
        let auth_peers = self.authorized_peers.read().await;
        match &*auth_peers {
            Some(whitelist) => whitelist.contains(peer_id),
            None => true, // No whitelist = all peers authorized
        }
    }

    pub async fn cache_block(&self, block: Block) {
        let mut cache = self.block_cache.lock().await;
        if cache.len() < MAX_BLOCK_CACHE_SIZE {
            cache.insert(block.hash, block);
        }
    }

    pub async fn add_peer(&self, peer: Peer) {
        let mut peers = self.known_peers.write().await;
        if peers.len() < MAX_KNOWN_PEERS {
            peers.insert(peer.id, peer.address);
        }
    }

    pub async fn resolve_fork(
        &self,
        peer_chain_state: &ChainState,
        local_chain_state: &ChainState,
        peer: &Peer,
    ) -> Result<u64, SyncError> {
        if peer_chain_state.latest_block_index <= local_chain_state.latest_block_index {
            return Ok(local_chain_state.latest_block_index);
        }

        let mut stream = self.connect_to_peer(peer.address).await?;

        let mut challenge = [0u8; 32];
        rand::rng().fill_bytes(&mut challenge);
        Self::send_message(
            &mut stream,
            NetworkMessage::Handshake {
                peer_id: self.peer_id,
                version: 1,
                challenge,
                listen_port: self.listen_addr.port(),
                chain_id: self.chain_id,
            },
        )
        .await?;

        let response = Self::receive_message(&mut stream).await?;
        match response {
            NetworkMessage::HandshakeAck {
                peer_id: remote_peer_id,
                signature,
                challenge: remote_challenge,
                chain_id: remote_chain_id,
                ..
            } => {
                if remote_chain_id != self.chain_id {
                    log::warn!(
                        "[P2P] Chain ID mismatch in resolve_fork: local={}, remote={}",
                        self.chain_id,
                        remote_chain_id
                    );
                    return Err(SyncError::SynchronizationError("Chain ID mismatch".to_string()));
                }
                let sig = Signature::from_slice(&signature)
                    .map_err(|_| SyncError::AuthenticationFailed)?;
                let remote_pk = ed25519_dalek::VerifyingKey::from_bytes(&remote_peer_id.to_bytes())
                    .map_err(|_| SyncError::AuthenticationFailed)?;
                remote_pk.verify(&challenge, &sig).map_err(|_| SyncError::AuthenticationFailed)?;

                if !self.is_peer_authorized(&remote_peer_id).await {
                    log::warn!(
                        "[SYNC] Peer {} is not authorized",
                        hex::encode(remote_peer_id.to_bytes())
                    );
                    return Err(SyncError::AuthenticationFailed);
                }

                let my_sig = {
                    let sig_key_guard = self.signing_key.lock().await;
                    let sig_key = sig_key_guard.as_ref().ok_or(SyncError::AuthenticationFailed)?;
                    sig_key.sign(&remote_challenge).to_vec()
                };
                Self::send_message(
                    &mut stream,
                    NetworkMessage::HandshakeVerify { signature: my_sig },
                )
                .await?;
            }
            _ => return Err(SyncError::AuthenticationFailed),
        }

        // Walk backwards to find common ancestor
        let local_height = local_chain_state.latest_block_index;
        let peer_height = peer_chain_state.latest_block_index;
        let search_from = std::cmp::min(local_height, peer_height);
        let search_limit = search_from.saturating_sub(MAX_BLOCK_RANGE);
        let mut common_ancestor_height = 0u64;

        for h in (search_limit..=search_from).rev() {
            Self::send_message(
                &mut stream,
                NetworkMessage::GetBlocks { from_height: h, to_height: h },
            )
            .await?;
            let resp = Self::receive_message(&mut stream).await?;
            let peer_block_hash = match resp {
                NetworkMessage::BlocksResponse { ref blocks } if !blocks.is_empty() => {
                    blocks[0].hash
                }
                _ => continue,
            };
            let local_block = {
                let storage_guard = self.storage.lock().await;
                storage_guard.as_ref().and_then(|s| s.get_block_by_height(h).ok().flatten())
            };
            if let Some(local_block) = local_block {
                if local_block.hash == peer_block_hash {
                    common_ancestor_height = h;
                    log::info!("Fork common ancestor at height {}", h);
                    break;
                }
            }
        }

        let fetch_from = common_ancestor_height + 1;
        Self::send_message(
            &mut stream,
            NetworkMessage::GetForkBlocks { from_height: fetch_from, to_height: peer_height },
        )
        .await?;

        let fork_response = Self::receive_message(&mut stream).await?;
        match fork_response {
            NetworkMessage::ForkBlocksResponse { blocks, total_height: _ } => {
                log::info!(
                    "Received {} fork blocks (heights {}-{})",
                    blocks.len(),
                    fetch_from,
                    peer_height
                );
                for block in &blocks {
                    self.cache_block(block.clone()).await;
                }
                if let Some(last) = blocks.last() {
                    Ok(last.index)
                } else {
                    Ok(local_chain_state.latest_block_index)
                }
            }
            _ => Err(SyncError::SynchronizationError("Failed to receive fork blocks".to_string())),
        }
    }

    pub fn detect_fork(
        &self,
        peer_head_hash: &[u8; 32],
        peer_height: u64,
        local_height: u64,
        local_head_hash: &[u8; 32],
    ) -> bool {
        peer_height == local_height && peer_head_hash != local_head_hash
    }

    pub async fn start_server(&self) -> Result<(), SyncError> {
        let mut running = self.is_running.lock().await;
        if *running {
            return Ok(());
        }
        *running = true;
        drop(running);

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        *self.shutdown_tx.lock().await = Some(shutdown_tx);

        let listener = TcpListener::bind(self.listen_addr)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        log::info!("P2P server listening on {}", self.listen_addr);

        let tls_acceptor: Option<tokio_rustls::TlsAcceptor> = self
            .tls_config
            .as_ref()
            .map(|tc| tokio_rustls::TlsAcceptor::from(Arc::new(tc.server_config.clone())));

        let semaphore = Arc::clone(&self.connection_semaphore);

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    let (socket, addr) = accept_result
                        .map_err(|e| SyncError::NetworkError(e.to_string()))?;

                    let peer_id = self.peer_id;
                    let peers = Arc::clone(&self.known_peers);
                    let block_cache = Arc::clone(&self.block_cache);
                    let tls_acceptor = tls_acceptor.clone();
                    let storage = Arc::clone(&self.storage);
                    let received_blocks = Arc::clone(&self.received_blocks);
                    let signing_key = Arc::clone(&self.signing_key);
                    let rate_limiters = Arc::clone(&self.peer_rate_limiters);
                    let listen_port = self.listen_addr.port();
                    let local_chain_id = self.chain_id;
                    let ann_tx = self.announcement_tx.clone();
                    let connection_timeout = self.connection_timeout;
                    let permit = semaphore.clone().acquire_owned().await;

                    let snapshots_dir = self.snapshots_dir.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        if let Some(acceptor) = tls_acceptor {
                            match acceptor.accept(socket).await {
                                Ok(tls_stream) => {
                                    if let Err(e) = Self::handle_connection(
                                        tls_stream, addr, peer_id, peers, block_cache, storage, received_blocks, signing_key, rate_limiters, listen_port, local_chain_id, ann_tx, snapshots_dir, connection_timeout,
                                    ).await {
                                        match e {
                                            SyncError::ConnectionTimeout => {
                                                log::debug!("TLS connection to {} closed after timeout", addr);
                                            }
                                            _ => {
                                                log::error!("TLS connection error: {}", e);
                                            }
                                        }
                                    }
                                }
                                Err(e) => log::error!("TLS handshake error from {}: {}", addr, e),
                            }
                        } else if let Err(e) = Self::handle_connection(
                            socket, addr, peer_id, peers, block_cache, storage, received_blocks, signing_key, rate_limiters, listen_port, local_chain_id, ann_tx, snapshots_dir, connection_timeout,
                        ).await {
                            match e {
                                SyncError::ConnectionTimeout => {
                                    log::debug!("Connection to {} closed after timeout", addr);
                                }
                                _ => {
                                    log::error!("Connection error: {}", e);
                                }
                            }
                        }
                    });
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        log::info!("P2P server shutting down");
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn stop_server(&self) {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(true);
        }
        let mut running = self.is_running.lock().await;
        *running = false;
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_connection<S>(
        mut socket: S,
        addr: SocketAddr,
        peer_id: PublicKey,
        peers: Arc<tokio::sync::RwLock<HashMap<PublicKey, SocketAddr>>>,
        block_cache: Arc<Mutex<HashMap<[u8; 32], Block>>>,
        storage: Arc<Mutex<Option<Box<dyn Storage>>>>,
        received_blocks: Arc<std::sync::Mutex<Vec<Block>>>,
        signing_key: Arc<Mutex<Option<SigningKey>>>,
        rate_limiters: Arc<Mutex<HashMap<SocketAddr, PerPeerRateLimiter>>>,
        listen_port: u16,
        local_chain_id: u64,
        announcement_tx: tokio::sync::broadcast::Sender<NetworkMessage>,
        snapshots_dir: Option<std::path::PathBuf>,
        connection_timeout: Duration,
    ) -> Result<(), SyncError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let inbound = timeout(
            connection_timeout,
            Self::receive_message_with_timeout(&mut socket, connection_timeout),
        )
        .await
        .map_err(|_| SyncError::ConnectionTimeout)??;

        match inbound {
            NetworkMessage::Handshake {
                peer_id: remote_peer_id,
                version,
                challenge,
                listen_port: remote_listen_port,
                chain_id: remote_chain_id,
            } => {
                if version != 1 {
                    return Err(SyncError::NetworkError("Version mismatch".to_string()));
                }
                if remote_chain_id != local_chain_id {
                    log::warn!(
                        "[P2P] Chain ID mismatch: local={}, remote={}, peer={}",
                        local_chain_id,
                        remote_chain_id,
                        addr
                    );
                    let mut my_challenge = [0u8; 32];
                    rand::RngCore::fill_bytes(&mut rand::rng(), &mut my_challenge);
                    let signature = {
                        let sig_key_guard = signing_key.lock().await;
                        sig_key_guard
                            .as_ref()
                            .map(|sk| sk.sign(&challenge).to_vec())
                            .unwrap_or_default()
                    };
                    let _ = Self::send_message(
                        &mut socket,
                        NetworkMessage::HandshakeAck {
                            peer_id,
                            version: 1,
                            signature,
                            challenge: my_challenge,
                            listen_port,
                            chain_id: local_chain_id,
                        },
                    )
                    .await;
                    return Err(SyncError::SynchronizationError("Chain ID mismatch".to_string()));
                }

                let mut my_challenge = [0u8; 32];
                rand::RngCore::fill_bytes(&mut rand::rng(), &mut my_challenge);

                let signature = {
                    let sig_key_guard = signing_key.lock().await;
                    let sig_key = sig_key_guard.as_ref().ok_or(SyncError::AuthenticationFailed)?;
                    sig_key.sign(&challenge).to_vec()
                };

                Self::send_message(
                    &mut socket,
                    NetworkMessage::HandshakeAck {
                        peer_id,
                        version: 1,
                        signature,
                        challenge: my_challenge,
                        listen_port,
                        chain_id: local_chain_id,
                    },
                )
                .await?;

                let verify = timeout(
                    connection_timeout,
                    Self::receive_message_with_timeout(&mut socket, connection_timeout),
                )
                .await
                .map_err(|_| SyncError::ConnectionTimeout)??;

                match verify {
                    NetworkMessage::HandshakeVerify { signature } => {
                        let sig = Signature::from_slice(&signature)
                            .map_err(|_| SyncError::AuthenticationFailed)?;
                        let remote_pk =
                            ed25519_dalek::VerifyingKey::from_bytes(&remote_peer_id.to_bytes())
                                .map_err(|_| SyncError::AuthenticationFailed)?;
                        remote_pk
                            .verify(&my_challenge, &sig)
                            .map_err(|_| SyncError::AuthenticationFailed)?;
                    }
                    _ => return Err(SyncError::AuthenticationFailed),
                }

                let peer_listen_addr = SocketAddr::new(addr.ip(), remote_listen_port);

                log::info!(
                    "Inbound peer authenticated: {} at {} (listen port {})",
                    hex::encode(remote_peer_id.to_bytes()),
                    addr,
                    remote_listen_port
                );

                {
                    let mut peers_guard = peers.write().await;
                    let stale: Vec<PublicKey> = peers_guard
                        .iter()
                        .filter(|(id, a)| **a == peer_listen_addr && **id != remote_peer_id)
                        .map(|(id, _)| *id)
                        .collect();
                    for id in stale {
                        peers_guard.remove(&id);
                    }
                    if peers_guard.len() < MAX_KNOWN_PEERS {
                        peers_guard.insert(remote_peer_id, peer_listen_addr);
                    }
                }
            }
            _ => return Err(SyncError::InvalidMessage),
        }
        log::debug!("[P2P] Inbound peer {} authenticated, entering message loop", addr);

        let mut announcement_rx = announcement_tx.subscribe();

        loop {
            {
                let mut rl_guard = rate_limiters.lock().await;
                let rl = rl_guard.entry(addr).or_insert_with(PerPeerRateLimiter::new);
                if !rl.allow() {
                    log::warn!("Rate limit exceeded for peer {}, disconnecting", addr);
                    break;
                }
            }

            let msg = tokio::select! {
                recv_result = Self::receive_message_with_timeout(&mut socket, connection_timeout) => {
                    match recv_result {
                        Ok(m) => m,
                        Err(SyncError::NetworkError(err_msg))
                            if err_msg.contains("early eof")
                                || err_msg.contains("connection reset")
                                || err_msg.contains("Connection reset") =>
                        {
                            log::debug!("Peer {} closed connection: {}", addr, err_msg);
                            break;
                        }
                        Err(SyncError::ConnectionTimeout) => {
                            log::debug!("Peer {} idle timeout, closing connection", addr);
                            break;
                        }
                        Err(e) => {
                            log::error!("Error receiving message from {}: {}", addr, e);
                            break;
                        }
                    }
                }
                announcement = announcement_rx.recv() => {
                    match announcement {
                        Ok(ann_msg) => {
                            let _ = Self::send_message(&mut socket, ann_msg).await;
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::debug!("[P2P] Announcement channel lagged by {} for {}", n, addr);
                            continue;
                        }
                        Err(_) => break,
                    }
                }
            };
            log::trace!("[P2P] Inbound msg from {}: {:?}", addr, std::mem::discriminant(&msg));
            match msg {
                NetworkMessage::PeerList { peers: peer_list } => {
                    let mut peers_guard = peers.write().await;
                    for (id, addr_str) in peer_list {
                        if peers_guard.len() >= MAX_KNOWN_PEERS {
                            break;
                        }
                        if let Ok(addr) = addr_str.parse() {
                            peers_guard.insert(id, addr);
                        }
                    }
                }
                NetworkMessage::RequestBlock { hash } => {
                    Self::handle_block_request_full(&mut socket, hash, &block_cache, &storage)
                        .await?;
                }
                NetworkMessage::BlockResponse { block } => {
                    Self::handle_block_response_full(block, &block_cache, &received_blocks).await;
                }
                NetworkMessage::NewBlockAnnouncement { block_hash, height } => {
                    log::debug!("[P2P] NewBlockAnnouncement height={} from {}", height, addr);
                    let already_applied = {
                        let storage_guard = storage.lock().await;
                        storage_guard
                            .as_ref()
                            .is_some_and(|s| s.get_block(&block_hash).ok().flatten().is_some())
                    };
                    if already_applied {
                        log::trace!("[P2P] Already have block {} in storage, skipping", height);
                        continue;
                    }

                    let (local_head_hash, local_height) =
                        Self::chain_head_from_storage(&block_cache, &storage).await;
                    log::debug!("[P2P] local_height={}, announced height={}", local_height, height);

                    if height > local_height.saturating_add(1) {
                        let from_height = if local_height > 0 {
                            local_height
                        } else {
                            local_height.saturating_add(1)
                        };
                        let to_height = height;
                        log::info!(
                            "Peer announced height {} (local {}), requesting backfill [{}..={}]",
                            height,
                            local_height,
                            from_height,
                            to_height
                        );
                        let send_result = Self::send_message(
                            &mut socket,
                            NetworkMessage::GetBlocks { from_height, to_height },
                        )
                        .await;
                        if let Err(e) = &send_result {
                            log::debug!("[P2P] GetBlocks send failed: {}", e);
                        }

                        match timeout(Duration::from_secs(10), Self::receive_message(&mut socket))
                            .await
                        {
                            Ok(Ok(NetworkMessage::BlocksResponse { mut blocks })) => {
                                log::debug!(
                                    "[P2P] Received {} backfill blocks from {}",
                                    blocks.len(),
                                    addr
                                );
                                blocks.sort_by_key(|b| b.index);

                                if blocks.first().is_some_and(|b| {
                                    b.index == local_height && b.hash == local_head_hash
                                }) {
                                    let _ = blocks.remove(0);
                                }

                                let mut accepted = 0usize;
                                for block in blocks {
                                    Self::handle_block_response_full(
                                        Some(block),
                                        &block_cache,
                                        &received_blocks,
                                    )
                                    .await;
                                    accepted = accepted.saturating_add(1);
                                }
                                log::info!(
                                    "Queued {} announced backfill blocks from peer",
                                    accepted
                                );
                            }
                            Ok(Ok(other)) => {
                                log::debug!(
                                    "Unexpected response while requesting announced backfill: {:?}",
                                    other
                                );
                            }
                            Ok(Err(e)) => {
                                log::debug!("Failed receiving announced backfill response: {}", e);
                            }
                            Err(_) => {
                                log::debug!("Timed out waiting for announced backfill response");
                            }
                        }
                    } else {
                        log::info!(
                            "Received announcement for unknown block {}, requesting it",
                            hex::encode(block_hash)
                        );
                        let _ = Self::send_message(
                            &mut socket,
                            NetworkMessage::RequestBlock { hash: block_hash },
                        )
                        .await;
                    }
                }
                NetworkMessage::GetChainHead => {
                    let (latest_hash, latest_height) =
                        Self::chain_head_from_storage(&block_cache, &storage).await;
                    Self::send_message(
                        &mut socket,
                        NetworkMessage::ChainHeadResponse {
                            latest_block_hash: latest_hash,
                            height: latest_height,
                        },
                    )
                    .await?;
                }
                NetworkMessage::GetBlocks { from_height, to_height } => {
                    let clamped_to = to_height.min(from_height.saturating_add(MAX_BLOCK_RANGE));
                    let blocks =
                        Self::blocks_in_range(&block_cache, &storage, from_height, clamped_to)
                            .await;
                    Self::send_message(&mut socket, NetworkMessage::BlocksResponse { blocks })
                        .await?;
                }
                NetworkMessage::GetForkBlocks { from_height, to_height } => {
                    let clamped_to = to_height.min(from_height.saturating_add(MAX_BLOCK_RANGE));
                    let blocks =
                        Self::blocks_in_range(&block_cache, &storage, from_height, clamped_to)
                            .await;
                    Self::send_message(
                        &mut socket,
                        NetworkMessage::ForkBlocksResponse {
                            blocks,
                            total_height: Self::chain_head_from_storage(&block_cache, &storage)
                                .await
                                .1,
                        },
                    )
                    .await?;
                }
                NetworkMessage::ForkResolution { common_height: _, fork_blocks } => {
                    for block in fork_blocks {
                        let mut cache = block_cache.lock().await;
                        if cache.len() < MAX_BLOCK_CACHE_SIZE {
                            cache.insert(block.hash, block.clone());
                        }
                        drop(cache);
                        let mut recv =
                            received_blocks.lock().expect("received_blocks mutex poisoned");
                        if recv.len() < MAX_RECEIVED_BLOCKS_QUEUE {
                            recv.push(block);
                        }
                    }
                }
                NetworkMessage::GetSnapshot { height } => {
                    let response =
                        Self::handle_get_snapshot(height, &storage, &snapshots_dir).await;
                    Self::send_message(&mut socket, response).await?;
                }
                NetworkMessage::QuorumSignatureRequest { block } => {
                    let response = Self::handle_quorum_signature_request(
                        block,
                        &storage,
                        &signing_key,
                        peer_id,
                        local_chain_id,
                    )
                    .await;
                    Self::send_message(&mut socket, response).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn send_message<S>(stream: &mut S, message: NetworkMessage) -> Result<(), SyncError>
    where
        S: AsyncWrite + Unpin,
    {
        const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;
        let serialized = bincode::serialize(&message)
            .map_err(|e| SyncError::SerializationError(e.to_string()))?;
        if serialized.len() > MAX_MESSAGE_SIZE {
            return Err(SyncError::InvalidMessage);
        }
        let length = (serialized.len() as u32).to_le_bytes();
        stream.write_all(&length).await.map_err(|e| SyncError::NetworkError(e.to_string()))?;
        stream.write_all(&serialized).await.map_err(|e| SyncError::NetworkError(e.to_string()))?;
        Ok(())
    }

    async fn receive_message<S>(stream: &mut S) -> Result<NetworkMessage, SyncError>
    where
        S: AsyncRead + Unpin,
    {
        Self::receive_message_with_timeout(stream, Duration::from_secs(10)).await
    }

    async fn receive_message_with_timeout<S>(
        stream: &mut S,
        io_timeout: Duration,
    ) -> Result<NetworkMessage, SyncError>
    where
        S: AsyncRead + Unpin,
    {
        let mut length_buffer = [0u8; 4];
        timeout(io_timeout, tokio::io::AsyncReadExt::read_exact(stream, &mut length_buffer))
            .await
            .map_err(|_| SyncError::ConnectionTimeout)?
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        let length = u32::from_le_bytes(length_buffer);
        const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024; // 16MB
        if length > MAX_MESSAGE_SIZE {
            return Err(SyncError::InvalidMessage);
        }

        let mut message_buffer = vec![0u8; length as usize];
        // Use a timeout for the body based on payload size while respecting minimum I/O timeout.
        let body_timeout = io_timeout.max(Duration::from_secs(10 + (length as u64 / 1_000_000)));
        timeout(body_timeout, tokio::io::AsyncReadExt::read_exact(stream, &mut message_buffer))
            .await
            .map_err(|_| SyncError::ConnectionTimeout)?
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        let message: NetworkMessage = bincode::deserialize(&message_buffer)
            .map_err(|e| SyncError::SerializationError(e.to_string()))?;
        Ok(message)
    }

    // Handle block request with storage fallback
    async fn handle_block_request_full<S>(
        stream: &mut S,
        hash: [u8; 32],
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
        storage: &Arc<Mutex<Option<Box<dyn Storage>>>>,
    ) -> Result<(), SyncError>
    where
        S: AsyncWrite + Unpin + Send,
    {
        // Try cache first
        let cached = {
            let cache = block_cache.lock().await;
            cache.get(&hash).cloned()
        };
        let block = if cached.is_some() {
            cached
        } else {
            // Try storage
            let storage_guard = storage.lock().await;
            storage_guard.as_ref().and_then(|s| s.get_block(&hash).ok().flatten())
        };
        Self::send_message(stream, NetworkMessage::BlockResponse { block }).await
    }

    // Handle block response: cache valid blocks and push to received queue
    async fn handle_block_response_full(
        block: Option<crate::types::Block>,
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
        received_blocks: &Arc<std::sync::Mutex<Vec<Block>>>,
    ) {
        if let Some(block) = block {
            if let Ok(calculated_hash) = block.calculate_hash() {
                if calculated_hash == block.hash {
                    let mut cache = block_cache.lock().await;
                    cache.insert(block.hash, block.clone());
                    drop(cache);
                    let mut recv = received_blocks.lock().expect("received_blocks mutex poisoned");
                    if recv.len() < MAX_RECEIVED_BLOCKS_QUEUE {
                        recv.push(block);
                    }
                }
            }
        }
    }

    async fn chain_head_from_storage(
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
        storage: &Arc<Mutex<Option<Box<dyn Storage>>>>,
    ) -> ([u8; 32], u64) {
        // Try storage first for accurate chain head
        let storage_guard = storage.lock().await;
        if let Some(s) = storage_guard.as_ref() {
            if let Ok(Some(cs)) = s.get_chain_state() {
                return (cs.latest_block_hash, cs.latest_block_index);
            }
        }
        drop(storage_guard);
        // Fall back to cache
        Self::chain_head_from_cache(block_cache).await
    }

    async fn chain_head_from_cache(
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
    ) -> ([u8; 32], u64) {
        let cache = block_cache.lock().await;
        if let Some(head) = cache.values().max_by_key(|block| block.index) {
            (head.hash, head.index)
        } else {
            ([0u8; 32], 0)
        }
    }

    async fn blocks_in_range(
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
        storage: &Arc<Mutex<Option<Box<dyn Storage>>>>,
        from_height: u64,
        to_height: u64,
    ) -> Vec<Block> {
        // Try storage first
        let storage_guard = storage.lock().await;
        if let Some(s) = storage_guard.as_ref() {
            if let Ok(hashes) = s.get_block_hashes_by_height_range(from_height, to_height) {
                let mut blocks = Vec::new();
                for hash in &hashes {
                    if let Ok(Some(block)) = s.get_block(hash) {
                        blocks.push(block);
                    }
                }
                if !blocks.is_empty() {
                    return blocks;
                }
            }
        }
        drop(storage_guard);
        // Fall back to cache
        let cache = block_cache.lock().await;
        let mut blocks: Vec<Block> = cache
            .values()
            .filter(|block| block.index >= from_height && block.index <= to_height)
            .cloned()
            .collect();
        blocks.sort_by_key(|block| block.index);
        blocks
    }

    /// Record a successful sync with a peer.
    async fn record_peer_success(&self, addr: SocketAddr) {
        let mut health = self.peer_health.lock().await;
        health.entry(addr).or_insert_with(PeerHealth::new).record_success();
    }

    /// Record a failed sync attempt with a peer.
    async fn record_peer_failure(&self, addr: SocketAddr) {
        let mut health = self.peer_health.lock().await;
        health.entry(addr).or_insert_with(PeerHealth::new).record_failure();
    }

    /// Check if a peer is in backoff. Returns true if we should skip this peer.
    async fn peer_in_backoff(&self, addr: SocketAddr) -> bool {
        let health = self.peer_health.lock().await;
        if let Some(h) = health.get(&addr) {
            h.backoff_remaining().is_some()
        } else {
            false
        }
    }

    /// Remove dead peers (too many consecutive failures) from known_peers.
    /// Also cleans up stale health entries for peers no longer in known_peers.
    pub async fn cleanup_dead_peers(&self) -> Vec<SocketAddr> {
        let dead: Vec<(PublicKey, SocketAddr)> = {
            let health = self.peer_health.lock().await;
            let peers = self.known_peers.read().await;
            peers
                .iter()
                .filter(|(_, addr)| health.get(addr).map(|h| h.is_dead()).unwrap_or(false))
                .map(|(id, addr)| (*id, *addr))
                .collect()
        };

        let mut removed = Vec::new();
        if !dead.is_empty() {
            let mut peers = self.known_peers.write().await;
            for (id, addr) in &dead {
                peers.remove(id);
                log::warn!(
                    "Removed dead peer {} ({} consecutive failures)",
                    addr,
                    PEER_MAX_CONSECUTIVE_FAILURES
                );
                removed.push(*addr);
            }
        }

        // Clean up health entries for peers no longer tracked
        {
            let peers = self.known_peers.read().await;
            let active_addrs: HashSet<SocketAddr> = peers.values().cloned().collect();
            let mut health = self.peer_health.lock().await;
            health.retain(|addr, _| active_addrs.contains(addr));
        }

        removed
    }

    /// Deduplicate peer entries: when we learn the real peer_id from a handshake,
    /// replace any dummy entry that has the same address.
    async fn dedup_peer(&self, real_peer_id: PublicKey, addr: SocketAddr) {
        let mut peers = self.known_peers.write().await;
        // Remove any entry with the same address but different (dummy) ID
        let stale_ids: Vec<PublicKey> = peers
            .iter()
            .filter(|(id, a)| **a == addr && **id != real_peer_id)
            .map(|(id, _)| *id)
            .collect();
        for id in stale_ids {
            peers.remove(&id);
        }
        // Ensure real ID → addr mapping exists
        if peers.len() < MAX_KNOWN_PEERS {
            peers.insert(real_peer_id, addr);
        }
    }

    /// When a fork is detected, request the peer's fork blocks starting from common ancestor.
    async fn resolve_fork_blocks<S: AsyncWrite + AsyncRead + Unpin + Send>(
        &self,
        _peer: &Peer,
        stream: &mut S,
        local_chain_state: &ChainState,
        peer_height: u64,
    ) -> Result<Block, SyncError> {
        let local_height = local_chain_state.latest_block_index;
        log::warn!("Resolving fork: local height={}, peer height={}", local_height, peer_height);

        // Walk backwards from min(local, peer) to find the common ancestor.
        // At each height, request that block from the peer and compare its hash
        // against our local block at the same height.
        let search_from = std::cmp::min(local_height, peer_height);
        let search_limit = search_from.saturating_sub(MAX_BLOCK_RANGE); // cap search depth
        let mut common_ancestor_height = 0u64; // genesis if nothing matches

        for h in (search_limit..=search_from).rev() {
            // Get the peer's block at this height
            Self::send_message(stream, NetworkMessage::GetBlocks { from_height: h, to_height: h })
                .await?;
            let resp = self.receive_sync_message(stream, Duration::from_secs(10)).await?;
            let peer_block_hash = match resp {
                NetworkMessage::BlocksResponse { ref blocks } if !blocks.is_empty() => {
                    blocks[0].hash
                }
                _ => continue,
            };

            // Compare with our local block at the same height
            let local_block = {
                let storage_guard = self.storage.lock().await;
                storage_guard.as_ref().and_then(|s| s.get_block_by_height(h).ok().flatten())
            };
            if let Some(local_block) = local_block {
                if local_block.hash == peer_block_hash {
                    common_ancestor_height = h;
                    log::info!(
                        "Fork common ancestor found at height {} (hash={})",
                        h,
                        crate::types::format_hex(&peer_block_hash)
                    );
                    break;
                }
            }
        }

        // Now fetch all peer blocks from common_ancestor+1 to peer_height
        let fetch_from = common_ancestor_height + 1;
        log::info!("Fetching fork blocks from height {} to {} from peer", fetch_from, peer_height);
        Self::send_message(
            stream,
            NetworkMessage::GetForkBlocks { from_height: fetch_from, to_height: peer_height },
        )
        .await?;

        let response = self.receive_sync_message(stream, Duration::from_secs(15)).await?;
        match response {
            NetworkMessage::ForkBlocksResponse { blocks, .. } => {
                log::info!(
                    "Received {} fork blocks from peer (heights {}-{})",
                    blocks.len(),
                    fetch_from,
                    peer_height
                );
                for block in &blocks {
                    let mut cache = self.block_cache.lock().await;
                    cache.insert(block.hash, block.clone());
                    drop(cache);
                    let mut recv =
                        self.received_blocks.lock().expect("received_blocks mutex poisoned");
                    if recv.len() < MAX_RECEIVED_BLOCKS_QUEUE {
                        recv.push(block.clone());
                    }
                }
                if let Some(last) = blocks.last() {
                    Ok(last.clone())
                } else {
                    Err(SyncError::SynchronizationError("No fork blocks received".to_string()))
                }
            }
            _ => Err(SyncError::SynchronizationError("Failed to receive fork blocks".to_string())),
        }
    }

    fn is_connection_error(err: &SyncError) -> bool {
        matches!(
            err,
            SyncError::NetworkError(_)
                | SyncError::SerializationError(_)
                | SyncError::ConnectionTimeout
                | SyncError::AuthenticationFailed
                | SyncError::InvalidMessage
        )
    }

    async fn perform_outbound_handshake<S>(
        &self,
        stream: &mut S,
        peer: &Peer,
    ) -> Result<(), SyncError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let mut challenge = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut challenge);

        Self::send_message(
            stream,
            NetworkMessage::Handshake {
                peer_id: self.peer_id,
                version: 1,
                challenge,
                listen_port: self.listen_addr.port(),
                chain_id: self.chain_id,
            },
        )
        .await?;

        let response = timeout(Duration::from_secs(10), Self::receive_message(stream))
            .await
            .map_err(|_| SyncError::ConnectionTimeout)??;

        match response {
            NetworkMessage::HandshakeAck {
                peer_id: remote_peer_id,
                signature,
                challenge: remote_challenge,
                listen_port: remote_listen_port,
                chain_id: remote_chain_id,
                ..
            } => {
                if remote_chain_id != self.chain_id {
                    log::warn!(
                        "[P2P] Chain ID mismatch: local={}, remote={}, peer={}",
                        self.chain_id,
                        remote_chain_id,
                        peer.address
                    );
                    return Err(SyncError::SynchronizationError("Chain ID mismatch".to_string()));
                }

                let sig = Signature::from_slice(&signature)
                    .map_err(|_| SyncError::AuthenticationFailed)?;
                let remote_pk = ed25519_dalek::VerifyingKey::from_bytes(&remote_peer_id.to_bytes())
                    .map_err(|_| SyncError::AuthenticationFailed)?;
                remote_pk.verify(&challenge, &sig).map_err(|_| SyncError::AuthenticationFailed)?;

                let my_sig = {
                    let sig_key_guard = self.signing_key.lock().await;
                    let sig_key = sig_key_guard.as_ref().ok_or(SyncError::AuthenticationFailed)?;
                    sig_key.sign(&remote_challenge).to_vec()
                };

                Self::send_message(stream, NetworkMessage::HandshakeVerify { signature: my_sig })
                    .await?;

                let peer_addr = SocketAddr::new(peer.address.ip(), remote_listen_port);
                self.dedup_peer(remote_peer_id, peer_addr).await;

                log::debug!(
                    "Outbound peer authenticated: {} at {}",
                    hex::encode(remote_peer_id.to_bytes()),
                    peer_addr
                );
                Ok(())
            }
            _ => Err(SyncError::AuthenticationFailed),
        }
    }

    async fn connect_and_authenticate_peer(
        &self,
        peer: &Peer,
    ) -> Result<Box<dyn AsyncStream>, SyncError> {
        let mut stream = self.connect_to_peer(peer.address).await?;
        self.perform_outbound_handshake(&mut stream, peer).await?;
        Ok(stream)
    }

    async fn receive_sync_message<S>(
        &self,
        stream: &mut S,
        timeout_duration: Duration,
    ) -> Result<NetworkMessage, SyncError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let deadline = tokio::time::Instant::now() + timeout_duration;

        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(SyncError::ConnectionTimeout);
            }
            let remaining = deadline.saturating_duration_since(now);

            let msg = timeout(remaining, Self::receive_message(stream))
                .await
                .map_err(|_| SyncError::ConnectionTimeout)??;

            match msg {
                NetworkMessage::GetSnapshot { height } => {
                    let response =
                        Self::handle_get_snapshot(height, &self.storage, &self.snapshots_dir).await;
                    Self::send_message(stream, response).await?;
                }
                NetworkMessage::RequestBlock { hash } => {
                    Self::handle_block_request_full(stream, hash, &self.block_cache, &self.storage)
                        .await?;
                }
                NetworkMessage::GetChainHead => {
                    let (latest_hash, latest_height) =
                        Self::chain_head_from_storage(&self.block_cache, &self.storage).await;
                    Self::send_message(
                        stream,
                        NetworkMessage::ChainHeadResponse {
                            latest_block_hash: latest_hash,
                            height: latest_height,
                        },
                    )
                    .await?;
                }
                NetworkMessage::GetBlocks { from_height, to_height } => {
                    let clamped_to = to_height.min(from_height.saturating_add(MAX_BLOCK_RANGE));
                    let blocks = Self::blocks_in_range(
                        &self.block_cache,
                        &self.storage,
                        from_height,
                        clamped_to,
                    )
                    .await;
                    Self::send_message(stream, NetworkMessage::BlocksResponse { blocks }).await?;
                }
                NetworkMessage::GetForkBlocks { from_height, to_height } => {
                    let clamped_to = to_height.min(from_height.saturating_add(MAX_BLOCK_RANGE));
                    let blocks = Self::blocks_in_range(
                        &self.block_cache,
                        &self.storage,
                        from_height,
                        clamped_to,
                    )
                    .await;
                    let (_, total_height) =
                        Self::chain_head_from_storage(&self.block_cache, &self.storage).await;
                    Self::send_message(
                        stream,
                        NetworkMessage::ForkBlocksResponse { blocks, total_height },
                    )
                    .await?;
                }
                NetworkMessage::Ping => {
                    let _ = Self::send_message(stream, NetworkMessage::Pong).await;
                }
                NetworkMessage::Pong => {}
                NetworkMessage::NewBlockAnnouncement { block_hash, height } => {
                    log::debug!(
                        "[P2P] Received async announcement while syncing: height={}, hash={}",
                        height,
                        crate::types::format_hex(&block_hash)
                    );
                }
                other => return Ok(other),
            }
        }
    }

    async fn sync_with_peer_stream<S>(
        &self,
        peer: &Peer,
        local_chain_state: &ChainState,
        stream: &mut S,
    ) -> Result<Block, SyncError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        Self::send_message(stream, NetworkMessage::GetChainHead).await?;
        let chain_head = self.receive_sync_message(stream, Duration::from_secs(10)).await?;

        match chain_head {
            NetworkMessage::ChainHeadResponse { latest_block_hash, height } => {
                log::debug!(
                    "[P2P] Outbound: peer {} height={} local={}",
                    peer.address,
                    height,
                    local_chain_state.latest_block_index
                );

                if local_chain_state.latest_block_index == 0 && height > 0 {
                    if let Some(ref dir) = self.snapshots_dir {
                        log::info!(
                            "[P2P] Attempting state snapshot sync from peer {} (local=0, peer={})",
                            peer.address,
                            height
                        );
                        if let Ok(_) =
                            Self::send_message(stream, NetworkMessage::GetSnapshot { height }).await
                        {
                            match self.receive_sync_message(stream, Duration::from_secs(30)).await {
                                Ok(NetworkMessage::SnapshotResponse {
                                    height: resp_height,
                                    snapshot: Some(snapshot_bytes),
                                    block: Some(block),
                                }) if resp_height == height => {
                                    log::info!(
                                        "[P2P] Received snapshot for height {} ({} bytes)",
                                        height,
                                        snapshot_bytes.len()
                                    );
                                    let snap_path = dir.join(format!("{}.snap", height));
                                    let _ = std::fs::create_dir_all(dir);
                                    if let Err(e) = std::fs::write(&snap_path, &snapshot_bytes) {
                                        log::error!("[P2P] Failed to write snapshot file: {:?}", e);
                                    }

                                    let restore_ok = {
                                        let storage_guard = self.storage.lock().await;
                                        if let Some(ref s) = *storage_guard {
                                            if let Err(e) = s.restore_from_snapshot(height, dir) {
                                                log::error!(
                                                    "[P2P] Failed to restore snapshot: {:?}",
                                                    e
                                                );
                                                false
                                            } else {
                                                if let Err(e) = s.put_block(&block) {
                                                    log::error!("[P2P] Failed to store snapshot block: {:?}", e);
                                                    false
                                                } else {
                                                    true
                                                }
                                            }
                                        } else {
                                            false
                                        }
                                    };

                                    if restore_ok {
                                        self.cache_block(block.clone()).await;
                                        log::info!("[P2P] Snapshot sync completed successfully at height {}", height);
                                        Self::serve_sync_requests(
                                            stream,
                                            &self.block_cache,
                                            &self.storage,
                                            self.snapshots_dir.clone(),
                                        )
                                        .await;
                                        return Ok(block);
                                    } else {
                                        log::warn!("[P2P] Snapshot restore failed, falling back to block sync");
                                    }
                                }
                                Ok(other) => {
                                    log::warn!(
                                        "[P2P] Unexpected response to GetSnapshot: {:?}. Falling back to block sync",
                                        other
                                    );
                                }
                                Err(e) => {
                                    log::warn!(
                                        "[P2P] Error receiving snapshot response: {:?}. Falling back to block sync",
                                        e
                                    );
                                }
                            }
                        }
                    } else {
                        log::info!("[P2P] Snapshot dir not configured, falling back to block sync");
                    }
                }

                if self.detect_fork(
                    &latest_block_hash,
                    height,
                    local_chain_state.latest_block_index,
                    &local_chain_state.latest_block_hash,
                ) {
                    log::warn!(
                        "FORK DETECTED at height {}: peer={} local={}",
                        height,
                        crate::types::format_hex(&latest_block_hash),
                        crate::types::format_hex(&local_chain_state.latest_block_hash)
                    );
                    let fork_result =
                        self.resolve_fork_blocks(peer, stream, local_chain_state, height).await;
                    Self::serve_sync_requests(
                        stream,
                        &self.block_cache,
                        &self.storage,
                        self.snapshots_dir.clone(),
                    )
                    .await;
                    return fork_result;
                }

                if height <= local_chain_state.latest_block_index {
                    log::info!(
                        "Peer is behind or equal (peer={}, local={}), announcing our chain head",
                        height,
                        local_chain_state.latest_block_index
                    );
                    let _ = Self::send_message(
                        stream,
                        NetworkMessage::NewBlockAnnouncement {
                            block_hash: local_chain_state.latest_block_hash,
                            height: local_chain_state.latest_block_index,
                        },
                    )
                    .await;
                    Self::serve_sync_requests_with_timeout(
                        stream,
                        &self.block_cache,
                        &self.storage,
                        self.snapshots_dir.clone(),
                        10,
                    )
                    .await;
                    return Err(SyncError::SynchronizationError(
                        "Peer is not ahead of local chain".to_string(),
                    ));
                }

                if height > local_chain_state.latest_block_index + 1 {
                    Self::send_message(
                        stream,
                        NetworkMessage::GetBlocks {
                            from_height: local_chain_state.latest_block_index + 1,
                            to_height: height,
                        },
                    )
                    .await?;
                    let blocks_response =
                        self.receive_sync_message(stream, Duration::from_secs(15)).await?;
                    match blocks_response {
                        NetworkMessage::BlocksResponse { blocks } => {
                            if blocks.is_empty() {
                                Self::serve_sync_requests(
                                    stream,
                                    &self.block_cache,
                                    &self.storage,
                                    self.snapshots_dir.clone(),
                                )
                                .await;
                                return Err(SyncError::SynchronizationError(
                                    "Empty blocks response".to_string(),
                                ));
                            }
                            if blocks[0].prev_hash != local_chain_state.latest_block_hash {
                                log::warn!(
                                    "Fork detected: received block #{} prev_hash doesn't match local head",
                                    blocks[0].index
                                );
                                let fork_result = self
                                    .resolve_fork_blocks(peer, stream, local_chain_state, height)
                                    .await;
                                Self::serve_sync_requests(
                                    stream,
                                    &self.block_cache,
                                    &self.storage,
                                    self.snapshots_dir.clone(),
                                )
                                .await;
                                return fork_result;
                            }
                            for block in &blocks {
                                let mut cache = self.block_cache.lock().await;
                                if cache.len() < MAX_BLOCK_CACHE_SIZE {
                                    cache.insert(block.hash, block.clone());
                                }
                            }
                            {
                                let mut recv = self
                                    .received_blocks
                                    .lock()
                                    .expect("received_blocks mutex poisoned");
                                if recv.len() + blocks.len() <= MAX_RECEIVED_BLOCKS_QUEUE {
                                    recv.extend(blocks.clone());
                                } else {
                                    log::warn!(
                                        "Dropped incoming blocks: received_blocks queue full"
                                    );
                                }
                            }
                            Self::serve_sync_requests(
                                stream,
                                &self.block_cache,
                                &self.storage,
                                self.snapshots_dir.clone(),
                            )
                            .await;
                            return Ok(blocks.last().unwrap().clone());
                        }
                        _ => {
                            Self::serve_sync_requests(
                                stream,
                                &self.block_cache,
                                &self.storage,
                                self.snapshots_dir.clone(),
                            )
                            .await;
                            return Err(SyncError::SynchronizationError(
                                "Unexpected response for GetBlocks".to_string(),
                            ));
                        }
                    }
                }

                Self::send_message(
                    stream,
                    NetworkMessage::RequestBlock { hash: latest_block_hash },
                )
                .await?;
                let block_response =
                    self.receive_sync_message(stream, Duration::from_secs(10)).await?;
                let block_result = match block_response {
                    NetworkMessage::BlockResponse { block: Some(block) } => {
                        if block.hash != latest_block_hash {
                            Err(SyncError::SynchronizationError(
                                "Peer returned unexpected block hash".to_string(),
                            ))
                        } else {
                            let mut cache = self.block_cache.lock().await;
                            cache.insert(block.hash, block.clone());
                            drop(cache);
                            let mut recv = self
                                .received_blocks
                                .lock()
                                .expect("received_blocks mutex poisoned");
                            if recv.len() < MAX_RECEIVED_BLOCKS_QUEUE {
                                recv.push(block.clone());
                            }
                            Ok(block)
                        }
                    }
                    NetworkMessage::BlockResponse { block: None } => Err(SyncError::BlockNotFound),
                    _ => Err(SyncError::InvalidMessage),
                };
                Self::serve_sync_requests(
                    stream,
                    &self.block_cache,
                    &self.storage,
                    self.snapshots_dir.clone(),
                )
                .await;
                block_result
            }
            _ => {
                Self::serve_sync_requests(
                    stream,
                    &self.block_cache,
                    &self.storage,
                    self.snapshots_dir.clone(),
                )
                .await;
                Err(SyncError::InvalidMessage)
            }
        }
    }

    /// Core sync-with-peer logic, called by the trait method which wraps it
    /// with health tracking and backoff checks.
    async fn sync_with_peer_inner(
        &self,
        peer: &Peer,
        local_chain_state: &ChainState,
    ) -> Result<Block, SyncError> {
        let existing_connection = {
            let mut connections = self.outbound_connections.lock().await;
            connections.remove(&peer.address)
        };
        let reused_connection = existing_connection.is_some();
        let mut stream = match existing_connection {
            Some(existing) => existing,
            None => self.connect_and_authenticate_peer(peer).await?,
        };

        let mut result = self.sync_with_peer_stream(peer, local_chain_state, &mut stream).await;

        if reused_connection && result.as_ref().err().is_some_and(Self::is_connection_error) {
            log::debug!("[SYNC] Reconnecting dropped persistent connection to {}", peer.address);
            stream = self.connect_and_authenticate_peer(peer).await?;
            result = self.sync_with_peer_stream(peer, local_chain_state, &mut stream).await;
        }

        let keep_connection = match &result {
            Ok(_) => true,
            Err(SyncError::SynchronizationError(msg)) if msg.contains("Chain ID mismatch") => false,
            Err(e) => !Self::is_connection_error(e),
        };
        if keep_connection {
            let mut connections = self.outbound_connections.lock().await;
            if connections.len() < MAX_KNOWN_PEERS {
                connections.insert(peer.address, stream);
            }
        }

        result
    }

    async fn send_announcement_to_peer(
        &self,
        peer: &Peer,
        announcement: &NetworkMessage,
    ) -> Result<(), SyncError> {
        let existing_connection = {
            let mut connections = self.outbound_connections.lock().await;
            connections.remove(&peer.address)
        };
        let reused_connection = existing_connection.is_some();
        let mut stream = match existing_connection {
            Some(existing) => existing,
            None => self.connect_and_authenticate_peer(peer).await?,
        };

        let mut send_result = Self::send_message(&mut stream, announcement.clone()).await;
        if reused_connection && send_result.as_ref().err().is_some_and(Self::is_connection_error) {
            log::debug!(
                "[P2P] Reconnecting outbound peer {} before announcement retry",
                peer.address
            );
            stream = self.connect_and_authenticate_peer(peer).await?;
            send_result = Self::send_message(&mut stream, announcement.clone()).await;
        }

        if send_result.is_ok() {
            Self::serve_sync_requests_with_timeout(
                &mut stream,
                &self.block_cache,
                &self.storage,
                self.snapshots_dir.clone(),
                2,
            )
            .await;
            let mut connections = self.outbound_connections.lock().await;
            if connections.len() < MAX_KNOWN_PEERS {
                connections.insert(peer.address, stream);
            }
        }

        send_result
    }

    async fn request_quorum_signature_from_peer(
        &self,
        peer: &Peer,
        block: &Block,
    ) -> Result<Option<(String, Vec<u8>)>, SyncError> {
        let mut stream = self.connect_and_authenticate_peer(peer).await?;
        Self::send_message(
            &mut stream,
            NetworkMessage::QuorumSignatureRequest { block: block.clone() },
        )
        .await?;

        let response = timeout(Duration::from_secs(10), Self::receive_message(&mut stream))
            .await
            .map_err(|_| SyncError::ConnectionTimeout)??;

        match response {
            NetworkMessage::QuorumSignatureResponse {
                accepted: true,
                signer: Some(signer),
                signature: Some(signature),
                ..
            } => Ok(Some((signer, signature))),
            NetworkMessage::QuorumSignatureResponse { accepted: false, reason, .. } => {
                if let Some(reason) = reason {
                    log::debug!(
                        "[P2P] Peer {} declined quorum signature: {}",
                        peer.address,
                        reason
                    );
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

#[async_trait]
impl SyncLayer for CustomSync {
    async fn sync_with_peer(
        &self,
        peer: &Peer,
        local_chain_state: &ChainState,
    ) -> Result<Block, SyncError> {
        // Skip peers in exponential backoff
        if self.peer_in_backoff(peer.address).await {
            log::debug!("[SYNC] Peer {} is in backoff; skipping", peer.address);
            return Err(SyncError::SynchronizationError("Peer in backoff".to_string()));
        }

        log::debug!("[SYNC] sync_with_peer_inner starting for {}", peer.address);
        let result = self.sync_with_peer_inner(peer, local_chain_state).await;

        match &result {
            Ok(_) => self.record_peer_success(peer.address).await,
            Err(SyncError::SynchronizationError(msg)) if msg.contains("Chain ID mismatch") => {
                self.record_peer_failure(peer.address).await;
                let mut peers = self.known_peers.write().await;
                peers.retain(|_, addr| *addr != peer.address);
                log::warn!("[P2P] Removed peer {} due to chain ID mismatch", peer.address);
            }
            Err(e) if !matches!(e, SyncError::SynchronizationError(_)) => {
                self.record_peer_failure(peer.address).await;
            }
            _ => {}
        }

        result
    }

    async fn discover_peers(&self) -> Result<Vec<Peer>, SyncError> {
        // Opportunistically clean up dead peers every discovery cycle
        self.cleanup_dead_peers().await;

        // Re-add bootstrap peers that were removed by cleanup
        let bootstrap_addrs: Vec<SocketAddr> =
            self.bootstrap_peers.lock().expect("bootstrap lock").clone();
        if !bootstrap_addrs.is_empty() {
            let active_addrs: HashSet<SocketAddr> = {
                let peers = self.known_peers.read().await;
                peers.values().cloned().collect()
            };
            let missing: Vec<SocketAddr> =
                bootstrap_addrs.iter().filter(|a| !active_addrs.contains(a)).cloned().collect();
            if !missing.is_empty() {
                let mut peers = self.known_peers.write().await;
                for addr in &missing {
                    let mut dummy_bytes = [0u8; 32];
                    rand::RngCore::fill_bytes(&mut rand::rng(), &mut dummy_bytes);
                    let dummy_sk = SigningKey::from_bytes(&dummy_bytes);
                    let pid = PublicKey::from(dummy_sk.verifying_key());
                    peers.insert(pid, *addr);
                    log::info!("[SYNC] Re-added bootstrap peer {}", addr);
                }
                drop(peers);
                let mut health = self.peer_health.lock().await;
                for addr in &missing {
                    health.remove(addr);
                }
            }
        }

        let peers = self.known_peers.read().await;
        Ok(peers.iter().map(|(id, addr)| Peer { id: *id, address: *addr }).collect())
    }

    async fn request_quorum_signatures(
        &self,
        block: &Block,
        peers: &[Peer],
    ) -> Result<Vec<(String, Vec<u8>)>, SyncError> {
        let mut collected = Vec::new();
        for peer in peers {
            if peer.address == self.listen_addr {
                continue;
            }
            match self.request_quorum_signature_from_peer(peer, block).await {
                Ok(Some(sig)) => collected.push(sig),
                Ok(None) => {}
                Err(e) => {
                    log::debug!("[P2P] Quorum signature request to {} failed: {}", peer.address, e);
                }
            }
        }
        Ok(collected)
    }

    async fn broadcast_block(&self, block: &Block, peers: &[Peer]) -> Result<(), SyncError> {
        self.cache_block(block.clone()).await;
        let announcement =
            NetworkMessage::NewBlockAnnouncement { block_hash: block.hash, height: block.index };

        // Push announcement to all inbound-connected peers via the broadcast channel.
        // then proactively announce to outbound peers as well.
        let receivers = self.announcement_tx.send(announcement.clone()).unwrap_or(0);
        let mut outbound_ok = 0usize;
        let mut outbound_failed = 0usize;
        for peer in peers {
            if peer.address == self.listen_addr {
                continue;
            }
            match self.send_announcement_to_peer(peer, &announcement).await {
                Ok(()) => outbound_ok = outbound_ok.saturating_add(1),
                Err(e) => {
                    outbound_failed = outbound_failed.saturating_add(1);
                    log::debug!("[P2P] Failed proactive announcement to {}: {}", peer.address, e);
                }
            }
        }
        log::info!(
            "[P2P] Broadcast block #{} (inbound peers={}, outbound ok={}, outbound failed={})",
            block.index,
            receivers,
            outbound_ok,
            outbound_failed
        );

        Ok(())
    }

    fn peer_count(&self) -> usize {
        self.known_peers.blocking_read().len()
    }

    async fn start_listener(&self) -> Result<(), SyncError> {
        self.start_server().await
    }

    fn add_peer_by_address(&self, addr: &str) -> Result<(), SyncError> {
        let sock_addr: SocketAddr = addr
            .parse()
            .map_err(|e| SyncError::NetworkError(format!("Invalid address '{}': {}", addr, e)))?;
        // Generate a valid Ed25519 keypair as placeholder; the real peer ID
        // is discovered during handshake authentication.
        let mut dummy_bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut dummy_bytes);
        let dummy_sk = SigningKey::from_bytes(&dummy_bytes);
        let peer_id = PublicKey::from(dummy_sk.verifying_key());
        self.known_peers.blocking_write().insert(peer_id, sock_addr);
        self.bootstrap_peers.lock().expect("bootstrap lock").push(sock_addr);
        log::info!("Added peer: {}", addr);
        Ok(())
    }

    fn poll_received_blocks(&self) -> Vec<Block> {
        let mut recv = self.received_blocks.lock().expect("received_blocks mutex poisoned");
        std::mem::take(&mut *recv)
    }

    fn remove_peer(&self, addr: &str) -> Result<(), SyncError> {
        let peers = self.known_peers.blocking_read();
        let target: Option<crate::types::PublicKey> =
            peers.iter().find(|(_, sock)| sock.to_string() == addr).map(|(pk, _)| *pk);
        drop(peers);
        if let Some(pk) = target {
            self.known_peers.blocking_write().remove(&pk);
            log::info!("Removed peer: {}", addr);
        }
        Ok(())
    }

    fn known_peers(&self) -> Vec<String> {
        self.known_peers.blocking_read().values().map(|sock| sock.to_string()).collect()
    }

    fn trigger_sync(&self) -> Result<(), SyncError> {
        log::info!("[SYNC] Manual sync triggered");
        Ok(())
    }

    fn stop_listener(&self) {
        let mut guard = self.shutdown_tx.blocking_lock();
        if let Some(tx) = guard.take() {
            let _ = tx.send(true);
            log::info!("P2P listener shutdown signal sent");
        }
        self.outbound_connections.blocking_lock().clear();
    }
}

impl CustomSync {
    /// Helper to process a `GetSnapshot` request and build the `SnapshotResponse`.
    async fn handle_get_snapshot(
        height: u64,
        storage: &Arc<Mutex<Option<Box<dyn Storage>>>>,
        snapshots_dir: &Option<std::path::PathBuf>,
    ) -> NetworkMessage {
        log::info!("[P2P] Processing GetSnapshot request for height {}", height);
        let mut snapshot_bytes = None;
        let mut block_data = None;

        if let Some(ref dir) = snapshots_dir {
            let block_opt = {
                let storage_guard = storage.lock().await;
                if let Some(ref s) = *storage_guard {
                    s.get_block_by_height(height).ok().flatten()
                } else {
                    None
                }
            };

            if let Some(block) = block_opt {
                block_data = Some(block);
                let snap_path = dir.join(format!("{}.snap", height));
                let exists = snap_path.exists();
                let snapshot_ok = if exists {
                    true
                } else {
                    let storage_guard = storage.lock().await;
                    if let Some(ref s) = *storage_guard {
                        match s.take_snapshot(height, dir) {
                            Ok(_) => true,
                            Err(e) => {
                                log::error!(
                                    "[P2P] Failed to take snapshot at height {}: {:?}",
                                    height,
                                    e
                                );
                                false
                            }
                        }
                    } else {
                        false
                    }
                };

                if snapshot_ok {
                    match std::fs::read(&snap_path) {
                        Ok(bytes) => {
                            snapshot_bytes = Some(bytes);
                        }
                        Err(e) => {
                            log::error!(
                                "[P2P] Failed to read snapshot file {:?}: {:?}",
                                snap_path,
                                e
                            );
                        }
                    }
                }
            } else {
                log::warn!("[P2P] GetSnapshot requested height {} but block not found", height);
            }
        } else {
            log::warn!("[P2P] GetSnapshot requested but snapshots_dir is not configured");
        }

        NetworkMessage::SnapshotResponse { height, snapshot: snapshot_bytes, block: block_data }
    }

    fn quorum_response_reject(reason: &str) -> NetworkMessage {
        NetworkMessage::QuorumSignatureResponse {
            accepted: false,
            signer: None,
            signature: None,
            reason: Some(reason.to_string()),
        }
    }

    fn block_signer_hex(block: &Block) -> Option<String> {
        if let Some(hex) = block.signer.as_ref() {
            return Some(hex.clone());
        }
        block.metadata.as_ref().and_then(|m| m.get("signer").cloned())
    }

    async fn handle_quorum_signature_request(
        block: Block,
        storage: &Arc<Mutex<Option<Box<dyn Storage>>>>,
        signing_key: &Arc<Mutex<Option<SigningKey>>>,
        local_peer_id: PublicKey,
        chain_id: u64,
    ) -> NetworkMessage {
        let signer_hex = match Self::block_signer_hex(&block) {
            Some(s) => s,
            None => return Self::quorum_response_reject("missing block signer metadata"),
        };

        let signer_bytes = match hex::decode(&signer_hex) {
            Ok(b) if b.len() == 32 => b,
            _ => return Self::quorum_response_reject("invalid signer public key"),
        };
        let mut signer_arr = [0u8; 32];
        signer_arr.copy_from_slice(&signer_bytes);
        let signer_pk = match PublicKey::from_bytes(&signer_arr) {
            Ok(pk) => pk,
            Err(_) => return Self::quorum_response_reject("invalid signer bytes"),
        };

        if signer_pk == local_peer_id {
            return Self::quorum_response_reject("cannot cosign own proposal");
        }

        let calculated_hash = match block.calculate_hash() {
            Ok(h) => h,
            Err(_) => return Self::quorum_response_reject("failed to calculate block hash"),
        };
        if calculated_hash != block.hash {
            return Self::quorum_response_reject("block hash mismatch");
        }

        if block.transactions.iter().any(|tx| tx.chain_id != chain_id) {
            return Self::quorum_response_reject("block chain_id does not match local chain");
        }

        let primary_sig_hex = match block.metadata.as_ref().and_then(|m| m.get("signature")) {
            Some(sig) => sig,
            None => return Self::quorum_response_reject("missing primary block signature"),
        };
        let primary_sig_bytes = match hex::decode(primary_sig_hex) {
            Ok(b) if b.len() == 64 => b,
            _ => return Self::quorum_response_reject("invalid primary signature encoding"),
        };
        let primary_sig = match Signature::from_slice(&primary_sig_bytes) {
            Ok(sig) => sig,
            Err(_) => return Self::quorum_response_reject("invalid primary signature format"),
        };
        if signer_pk.verify(&block.hash, &primary_sig).is_err() {
            return Self::quorum_response_reject("primary signature verification failed");
        }

        let storage_guard = storage.lock().await;
        let Some(storage_ref) = storage_guard.as_ref() else {
            return Self::quorum_response_reject("local storage unavailable");
        };

        let chain_state = match storage_ref.get_chain_state() {
            Ok(Some(cs)) => cs,
            Ok(None) => return Self::quorum_response_reject("local chain state unavailable"),
            Err(_) => return Self::quorum_response_reject("failed to load local chain state"),
        };

        if block.index != chain_state.latest_block_index.saturating_add(1) {
            return Self::quorum_response_reject("block height not next to local head");
        }
        if block.prev_hash != chain_state.latest_block_hash {
            return Self::quorum_response_reject("block prev_hash does not match local head");
        }

        let authorized_signers = match storage_ref.get_authorized_signers() {
            Ok(s) => s,
            Err(_) => return Self::quorum_response_reject("failed to load authorized signers"),
        };
        if !authorized_signers.is_empty() && !authorized_signers.contains(&signer_pk) {
            return Self::quorum_response_reject("proposer is not in local authorized signer set");
        }

        drop(storage_guard);

        let signer_key_guard = signing_key.lock().await;
        let Some(local_signing_key) = signer_key_guard.as_ref() else {
            return Self::quorum_response_reject("local signing key unavailable");
        };
        let sig = local_signing_key.sign(&block.hash).to_bytes().to_vec();
        let local_signer_hex = hex::encode(local_peer_id.to_bytes());
        NetworkMessage::QuorumSignatureResponse {
            accepted: true,
            signer: Some(local_signer_hex),
            signature: Some(sig),
            reason: None,
        }
    }

    /// Serve pending sync requests briefly on an outbound connection before closing.
    /// This allows the server's bidirectional sync to request backfill blocks.
    async fn serve_sync_requests<S: AsyncRead + AsyncWrite + Unpin + Send>(
        stream: &mut S,
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
        storage: &Arc<Mutex<Option<Box<dyn Storage>>>>,
        snapshots_dir: Option<std::path::PathBuf>,
    ) {
        Self::serve_sync_requests_with_timeout(stream, block_cache, storage, snapshots_dir, 2)
            .await;
    }

    async fn serve_sync_requests_with_timeout<S: AsyncRead + AsyncWrite + Unpin + Send>(
        stream: &mut S,
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
        storage: &Arc<Mutex<Option<Box<dyn Storage>>>>,
        snapshots_dir: Option<std::path::PathBuf>,
        timeout_secs: u64,
    ) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(now);
            let incoming = timeout(remaining, Self::receive_message(stream)).await;
            match incoming {
                Ok(Ok(NetworkMessage::GetChainHead)) => {
                    let (lh, lh2) = Self::chain_head_from_storage(block_cache, storage).await;
                    let _ = Self::send_message(
                        stream,
                        NetworkMessage::ChainHeadResponse { latest_block_hash: lh, height: lh2 },
                    )
                    .await;
                }
                Ok(Ok(NetworkMessage::GetBlocks { from_height, to_height })) => {
                    let clamped_to = to_height.min(from_height.saturating_add(MAX_BLOCK_RANGE));
                    let blocks =
                        Self::blocks_in_range(block_cache, storage, from_height, clamped_to).await;
                    let count = blocks.len();
                    log::trace!(
                        "[P2P] serve_sync: serving {} blocks ({}..={})",
                        count,
                        from_height,
                        clamped_to
                    );
                    let _ =
                        Self::send_message(stream, NetworkMessage::BlocksResponse { blocks }).await;
                }
                Ok(Ok(NetworkMessage::RequestBlock { hash })) => {
                    log::trace!("[P2P] serve_sync: serving block request {}", hex::encode(hash));
                    let _ =
                        Self::handle_block_request_full(stream, hash, block_cache, storage).await;
                }
                Ok(Ok(NetworkMessage::GetSnapshot { height })) => {
                    let response = Self::handle_get_snapshot(height, storage, &snapshots_dir).await;
                    let _ = Self::send_message(stream, response).await;
                }
                Ok(Ok(other)) => {
                    log::trace!("[P2P] serve_sync: ignoring {:?}", std::mem::discriminant(&other));
                }
                Err(_) => break,
                Ok(Err(e)) => {
                    log::debug!("[P2P] serve_sync: receive error: {}", e);
                    break;
                }
            }
        }
    }

    async fn connect_to_peer(
        &self,
        address: SocketAddr,
    ) -> Result<Box<dyn AsyncStream>, SyncError> {
        let stream = timeout(Duration::from_secs(5), TcpStream::connect(address))
            .await
            .map_err(|_| SyncError::ConnectionTimeout)?
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        if let Some(tls) = &self.tls_config {
            let client_config = if self.tls_insecure {
                tokio_rustls::rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(CertificatePinner::new(Arc::new(
                        RwLock::new(HashSet::new()),
                    ))))
                    .with_no_client_auth()
            } else {
                tls.client_config.clone()
            };
            let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));
            let domain = rustls::pki_types::ServerName::try_from("baals-node")
                .map_err(|e| SyncError::NetworkError(e.to_string()))?;
            let tls_stream = connector.connect(domain, stream).await.map_err(|e| {
                SyncError::NetworkError(format!("TLS connect to {}: {}", address, e))
            })?;
            Ok(Box::new(tls_stream))
        } else {
            Ok(Box::new(stream))
        }
    }
}

#[cfg(feature = "mdns")]
pub mod discovery {
    use super::*;
    use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
    use std::collections::HashSet;

    pub struct MdnsDiscovery {
        daemon: ServiceDaemon,
        service_name: String,
        listen_port: u16,
        node_id: PublicKey,
    }

    impl MdnsDiscovery {
        pub fn new(node_id: PublicKey, listen_port: u16) -> Result<Self, SyncError> {
            let daemon = ServiceDaemon::new()
                .map_err(|e| SyncError::NetworkError(format!("mDNS daemon: {}", e)))?;
            Ok(Self {
                daemon,
                service_name: "_baals._tcp.local.".to_string(),
                listen_port,
                node_id,
            })
        }

        pub fn start_announcing(&self) -> Result<(), SyncError> {
            let node_id_hex = hex::encode(self.node_id.to_bytes());
            let properties = [("node_id", node_id_hex.as_str())];
            let service_info = ServiceInfo::new(
                &self.service_name,
                &node_id_hex,
                "baals.local.",
                (),
                self.listen_port,
                &properties[..],
            )
            .map(|svc| svc.enable_addr_auto())
            .map_err(|e| SyncError::NetworkError(format!("mDNS register: {}", e)))?;
            self.daemon
                .register(service_info)
                .map_err(|e| SyncError::NetworkError(format!("mDNS register: {}", e)))?;
            log::info!("mDNS: announcing BaaLS node {} on port {}", node_id_hex, self.listen_port);
            Ok(())
        }

        pub fn browse(&self) -> Result<Vec<(String, u16)>, SyncError> {
            let receiver = self
                .daemon
                .browse(&self.service_name)
                .map_err(|e| SyncError::NetworkError(format!("mDNS browse: {}", e)))?;
            let mut discovered = HashSet::new();
            while let Ok(event) = receiver.recv_timeout(std::time::Duration::from_millis(500)) {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let port = info.get_port();
                        for addr in info.get_addresses() {
                            discovered.insert((addr.to_string(), port));
                        }
                    }
                    ServiceEvent::SearchStarted(_) => {}
                    _ => break,
                }
            }
            Ok(discovered.into_iter().collect())
        }
    }
}

/// No-operation implementation for testing
#[derive(Debug, Clone)]
pub struct NoopSync;

#[async_trait]
impl SyncLayer for NoopSync {
    async fn sync_with_peer(
        &self,
        _peer: &Peer,
        _local_chain_state: &ChainState,
    ) -> Result<Block, SyncError> {
        Err(SyncError::SynchronizationError(
            "No-op sync does not perform actual synchronization".to_string(),
        ))
    }

    async fn discover_peers(&self) -> Result<Vec<Peer>, SyncError> {
        Ok(vec![])
    }

    async fn broadcast_block(&self, _block: &Block, _peers: &[Peer]) -> Result<(), SyncError> {
        Ok(())
    }

    async fn request_quorum_signatures(
        &self,
        _block: &Block,
        _peers: &[Peer],
    ) -> Result<Vec<(String, Vec<u8>)>, SyncError> {
        Ok(Vec::new())
    }
}

/// Runtime-switchable sync layer wrapper.
#[derive(Debug)]
pub enum SyncWrapper {
    Noop(NoopSync),
    Custom(Box<CustomSync>),
}

impl Clone for SyncWrapper {
    fn clone(&self) -> Self {
        match self {
            SyncWrapper::Noop(_) => SyncWrapper::Noop(NoopSync),
            SyncWrapper::Custom(cs) => SyncWrapper::Custom(Box::new(CustomSync {
                peer_id: cs.peer_id,
                known_peers: Arc::clone(&cs.known_peers),
                authorized_peers: Arc::clone(&cs.authorized_peers),
                block_cache: Arc::clone(&cs.block_cache),
                listen_addr: cs.listen_addr,
                is_running: Arc::clone(&cs.is_running),
                tls_config: cs.tls_config.clone(),
                tls_insecure: cs.tls_insecure,
                storage: Arc::clone(&cs.storage),
                received_blocks: Arc::clone(&cs.received_blocks),
                shutdown_tx: Arc::clone(&cs.shutdown_tx),
                connection_semaphore: Arc::clone(&cs.connection_semaphore),
                signing_key: Arc::clone(&cs.signing_key),
                peer_rate_limiters: Arc::clone(&cs.peer_rate_limiters),
                peer_health: Arc::clone(&cs.peer_health),
                outbound_connections: Arc::clone(&cs.outbound_connections),
                chain_id: cs.chain_id,
                announcement_tx: cs.announcement_tx.clone(),
                bootstrap_peers: Arc::clone(&cs.bootstrap_peers),
                snapshots_dir: cs.snapshots_dir.clone(),
                connection_timeout: cs.connection_timeout,
            })),
        }
    }
}

#[async_trait]
impl SyncLayer for SyncWrapper {
    async fn sync_with_peer(
        &self,
        peer: &Peer,
        local_chain_state: &ChainState,
    ) -> Result<Block, SyncError> {
        match self {
            SyncWrapper::Noop(n) => n.sync_with_peer(peer, local_chain_state).await,
            SyncWrapper::Custom(c) => c.sync_with_peer(peer, local_chain_state).await,
        }
    }

    async fn discover_peers(&self) -> Result<Vec<Peer>, SyncError> {
        match self {
            SyncWrapper::Noop(n) => n.discover_peers().await,
            SyncWrapper::Custom(c) => c.discover_peers().await,
        }
    }

    async fn broadcast_block(&self, block: &Block, peers: &[Peer]) -> Result<(), SyncError> {
        match self {
            SyncWrapper::Noop(n) => n.broadcast_block(block, peers).await,
            SyncWrapper::Custom(c) => c.broadcast_block(block, peers).await,
        }
    }

    async fn request_quorum_signatures(
        &self,
        block: &Block,
        peers: &[Peer],
    ) -> Result<Vec<(String, Vec<u8>)>, SyncError> {
        match self {
            SyncWrapper::Noop(n) => n.request_quorum_signatures(block, peers).await,
            SyncWrapper::Custom(c) => c.request_quorum_signatures(block, peers).await,
        }
    }

    fn peer_count(&self) -> usize {
        match self {
            SyncWrapper::Noop(n) => n.peer_count(),
            SyncWrapper::Custom(c) => c.peer_count(),
        }
    }

    async fn start_listener(&self) -> Result<(), SyncError> {
        match self {
            SyncWrapper::Noop(n) => n.start_listener().await,
            SyncWrapper::Custom(c) => c.start_listener().await,
        }
    }

    fn stop_listener(&self) {
        match self {
            SyncWrapper::Noop(n) => n.stop_listener(),
            SyncWrapper::Custom(c) => c.stop_listener(),
        }
    }

    fn add_peer_by_address(&self, addr: &str) -> Result<(), SyncError> {
        match self {
            SyncWrapper::Noop(n) => n.add_peer_by_address(addr),
            SyncWrapper::Custom(c) => c.add_peer_by_address(addr),
        }
    }

    fn remove_peer(&self, addr: &str) -> Result<(), SyncError> {
        match self {
            SyncWrapper::Noop(n) => n.remove_peer(addr),
            SyncWrapper::Custom(c) => c.remove_peer(addr),
        }
    }

    fn known_peers(&self) -> Vec<String> {
        match self {
            SyncWrapper::Noop(n) => n.known_peers(),
            SyncWrapper::Custom(c) => c.known_peers(),
        }
    }

    fn trigger_sync(&self) -> Result<(), SyncError> {
        match self {
            SyncWrapper::Noop(n) => n.trigger_sync(),
            SyncWrapper::Custom(c) => c.trigger_sync(),
        }
    }

    fn poll_received_blocks(&self) -> Vec<Block> {
        match self {
            SyncWrapper::Noop(n) => n.poll_received_blocks(),
            SyncWrapper::Custom(c) => c.poll_received_blocks(),
        }
    }
}
