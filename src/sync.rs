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
    Handshake { peer_id: PublicKey, version: u32, challenge: [u8; 32] },
    HandshakeAck { peer_id: PublicKey, version: u32, signature: Vec<u8>, challenge: [u8; 32] },
    HandshakeVerify { signature: Vec<u8> },

    // Sync protocol messages
    GetChainHead,
    ChainHeadResponse { latest_block_hash: [u8; 32], height: u64 },
    GetBlocks { from_height: u64, to_height: u64 },
    BlocksResponse { blocks: Vec<Block> },
    NewBlockAnnouncement { block_hash: [u8; 32], height: u64 },

    // Fork resolution
    ForkResolution { common_height: u64, fork_blocks: Vec<Block> },
    GetForkBlocks { from_height: u64, to_height: u64 },
    ForkBlocksResponse { blocks: Vec<Block>, total_height: u64 },

    // Keep-alive
    Ping,
    Pong,
    PeerList { peers: Vec<(PublicKey, String)> },
    RequestBlock { hash: [u8; 32] },
    BlockResponse { block: Option<crate::types::Block> },
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

/// Minimal custom P2P sync implementation
pub struct CustomSync {
    peer_id: PublicKey,
    known_peers: Arc<tokio::sync::RwLock<HashMap<PublicKey, SocketAddr>>>,
    block_cache: Arc<Mutex<HashMap<[u8; 32], Block>>>,
    listen_addr: SocketAddr,
    is_running: Arc<Mutex<bool>>,
    tls_config: Option<Arc<TlsConfig>>,
    tls_insecure: bool,
    storage: Arc<Mutex<Option<Box<dyn Storage>>>>,
    received_blocks: Arc<Mutex<Vec<Block>>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
    connection_semaphore: Arc<tokio::sync::Semaphore>,
    signing_key: Arc<Mutex<Option<SigningKey>>>,
    peer_rate_limiters: Arc<Mutex<HashMap<SocketAddr, PerPeerRateLimiter>>>,
}

const MAX_CONCURRENT_CONNECTIONS: usize = 128;
const MAX_KNOWN_PEERS: usize = 2048;
const MAX_RECEIVED_BLOCKS_QUEUE: usize = 1000;
const MAX_BLOCK_CACHE_SIZE: usize = 5000;
const MAX_P2P_MESSAGES_PER_SECOND: u32 = 50;
const MAX_BLOCK_RANGE: u64 = 500;

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

        // If a CA certificate path is provided, load and pin the CA certificate(s)
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
                        "Pinned CA certificate with SHA256 fingerprint: {}",
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
        _intermediates: &[rustls::pki_types::CertificateDer],
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
        let cert_hash = Sha256::digest(end_entity.as_ref()).to_vec();
        if pins.contains(&cert_hash) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("certificate fingerprint not in pin set".into()))
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
    pub fn new(peer_id: PublicKey, listen_addr: SocketAddr) -> Self {
        Self {
            peer_id,
            known_peers: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            block_cache: Arc::new(Mutex::new(HashMap::new())),
            listen_addr,
            is_running: Arc::new(Mutex::new(false)),
            tls_config: None,
            tls_insecure: false,
            storage: Arc::new(Mutex::new(None)),
            received_blocks: Arc::new(Mutex::new(Vec::new())),
            shutdown_tx: Arc::new(Mutex::new(None)),
            connection_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONNECTIONS)),
            signing_key: Arc::new(Mutex::new(None)),
            peer_rate_limiters: Arc::new(Mutex::new(HashMap::new())),
        }
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
            NetworkMessage::Handshake { peer_id: self.peer_id, version: 1, challenge },
        )
        .await?;

        let response = Self::receive_message(&mut stream).await?;
        match response {
            NetworkMessage::HandshakeAck {
                peer_id: remote_peer_id,
                signature,
                challenge: remote_challenge,
                ..
            } => {
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
                Self::send_message(
                    &mut stream,
                    NetworkMessage::HandshakeVerify { signature: my_sig },
                )
                .await?;
            }
            _ => return Err(SyncError::AuthenticationFailed),
        }

        let common_height = local_chain_state.latest_block_index;
        Self::send_message(
            &mut stream,
            NetworkMessage::GetForkBlocks {
                from_height: common_height + 1,
                to_height: peer_chain_state.latest_block_index,
            },
        )
        .await?;

        let fork_response = Self::receive_message(&mut stream).await?;
        match fork_response {
            NetworkMessage::ForkBlocksResponse { blocks, total_height: _ } => {
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
                    let permit = semaphore.clone().acquire_owned().await;

                    tokio::spawn(async move {
                        let _permit = permit;
                        if let Some(acceptor) = tls_acceptor {
                            match acceptor.accept(socket).await {
                                Ok(tls_stream) => {
                                    if let Err(e) = Self::handle_connection(
                                        tls_stream, addr, peer_id, peers, block_cache, storage, received_blocks, signing_key, rate_limiters,
                                    ).await {
                                        log::error!("TLS connection error: {}", e);
                                    }
                                }
                                Err(e) => log::error!("TLS handshake error from {}: {}", addr, e),
                            }
                        } else if let Err(e) = Self::handle_connection(
                            socket, addr, peer_id, peers, block_cache, storage, received_blocks, signing_key, rate_limiters,
                        ).await {
                            log::error!("Connection error: {}", e);
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
        received_blocks: Arc<Mutex<Vec<Block>>>,
        signing_key: Arc<Mutex<Option<SigningKey>>>,
        rate_limiters: Arc<Mutex<HashMap<SocketAddr, PerPeerRateLimiter>>>,
    ) -> Result<(), SyncError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        // Expect inbound handshake from peer, then ack with signature of their challenge.
        let inbound = timeout(Duration::from_secs(30), Self::receive_message(&mut socket))
            .await
            .map_err(|_| SyncError::ConnectionTimeout)??;

        match inbound {
            NetworkMessage::Handshake { peer_id: remote_peer_id, version, challenge } => {
                if version != 1 {
                    return Err(SyncError::NetworkError("Version mismatch".to_string()));
                }
                // Sign their challenge and send our own
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
                    },
                )
                .await?;

                // Expect verification of our challenge
                let verify = timeout(Duration::from_secs(10), Self::receive_message(&mut socket))
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

                log::info!(
                    "Inbound peer connected and mutually authenticated: {} at {}",
                    hex::encode(remote_peer_id.to_bytes()),
                    addr
                );
            }
            _ => return Err(SyncError::InvalidMessage),
        }
        // After handshake, enter message loop
        loop {
            // Per-peer rate limit: max MAX_P2P_MESSAGES_PER_SECOND messages/sec
            {
                let mut rl_guard = rate_limiters.lock().await;
                let rl = rl_guard.entry(addr).or_insert_with(PerPeerRateLimiter::new);
                if !rl.allow() {
                    log::warn!("Rate limit exceeded for peer {}, disconnecting", addr);
                    break;
                }
            }

            let msg = match Self::receive_message(&mut socket).await {
                Ok(m) => m,
                Err(SyncError::NetworkError(err_msg))
                    if err_msg.contains("early eof")
                        || err_msg.contains("connection reset")
                        || err_msg.contains("Connection reset") =>
                {
                    log::debug!("Peer {} closed connection: {}", addr, err_msg);
                    break;
                }
                Err(e) => {
                    log::error!("Error receiving message from {}: {}", addr, e);
                    break;
                }
            };
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
                NetworkMessage::NewBlockAnnouncement { block_hash, height: _ } => {
                    // Check if we already have this block
                    let have_it = {
                        let cache = block_cache.lock().await;
                        cache.contains_key(&block_hash)
                    } || {
                        let storage_guard = storage.lock().await;
                        storage_guard
                            .as_ref()
                            .is_some_and(|s| s.get_block(&block_hash).ok().flatten().is_some())
                    };
                    if !have_it {
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
                        let mut recv = received_blocks.lock().await;
                        if recv.len() < MAX_RECEIVED_BLOCKS_QUEUE {
                            recv.push(block);
                        }
                    }
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
        let mut length_buffer = [0u8; 4];
        // Use a reasonable timeout for the header
        timeout(
            Duration::from_secs(10),
            tokio::io::AsyncReadExt::read_exact(stream, &mut length_buffer),
        )
        .await
        .map_err(|_| SyncError::ConnectionTimeout)?
        .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        let length = u32::from_le_bytes(length_buffer);
        const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024; // 16MB
        if length > MAX_MESSAGE_SIZE {
            return Err(SyncError::InvalidMessage);
        }

        let mut message_buffer = vec![0u8; length as usize];
        // Use a timeout for the body based on the expected size (min 10s)
        let body_timeout = Duration::from_secs(10 + (length as u64 / 1_000_000));
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
        received_blocks: &Arc<Mutex<Vec<Block>>>,
    ) {
        if let Some(block) = block {
            if let Ok(calculated_hash) = block.calculate_hash() {
                if calculated_hash == block.hash {
                    let mut cache = block_cache.lock().await;
                    cache.insert(block.hash, block.clone());
                    drop(cache);
                    let mut recv = received_blocks.lock().await;
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

    /// When a fork is detected, request the peer's fork blocks starting from common ancestor.
    async fn resolve_fork_blocks<S: AsyncWrite + AsyncRead + Unpin + Send>(
        &self,
        _peer: &Peer,
        stream: &mut S,
        local_chain_state: &ChainState,
        peer_height: u64,
    ) -> Result<Block, SyncError> {
        log::warn!(
            "Resolving fork: local height={}, peer height={}",
            local_chain_state.latest_block_index,
            peer_height
        );
        let common_height = local_chain_state.latest_block_index;
        Self::send_message(
            stream,
            NetworkMessage::GetForkBlocks {
                from_height: common_height + 1,
                to_height: peer_height,
            },
        )
        .await?;

        let response = Self::receive_message(stream).await?;
        match response {
            NetworkMessage::ForkBlocksResponse { blocks, .. } => {
                log::info!("Received {} fork blocks from peer", blocks.len());
                for block in &blocks {
                    let mut cache = self.block_cache.lock().await;
                    cache.insert(block.hash, block.clone());
                    drop(cache);
                    let mut recv = self.received_blocks.lock().await;
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
}

#[async_trait]
impl SyncLayer for CustomSync {
    async fn sync_with_peer(
        &self,
        peer: &Peer,
        local_chain_state: &ChainState,
    ) -> Result<Block, SyncError> {
        let mut stream = self.connect_to_peer(peer.address).await?;

        // Perform handshake with challenge
        let mut challenge = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut challenge);

        Self::send_message(
            &mut stream,
            NetworkMessage::Handshake { peer_id: self.peer_id, version: 1, challenge },
        )
        .await?;

        let response = Self::receive_message(&mut stream).await?;
        match response {
            NetworkMessage::HandshakeAck {
                peer_id: remote_peer_id,
                signature,
                challenge: remote_challenge,
                ..
            } => {
                // 1. Verify their signature of our challenge
                let sig = Signature::from_slice(&signature)
                    .map_err(|_| SyncError::AuthenticationFailed)?;
                let remote_pk = ed25519_dalek::VerifyingKey::from_bytes(&remote_peer_id.to_bytes())
                    .map_err(|_| SyncError::AuthenticationFailed)?;
                remote_pk.verify(&challenge, &sig).map_err(|_| SyncError::AuthenticationFailed)?;

                // 2. Sign their challenge
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

                log::info!(
                    "Outbound peer mutually authenticated: {}",
                    hex::encode(remote_peer_id.to_bytes())
                );
            }
            _ => return Err(SyncError::AuthenticationFailed),
        }

        // Request chain head
        Self::send_message(&mut stream, NetworkMessage::GetChainHead).await?;
        let chain_head = Self::receive_message(&mut stream).await?;

        match chain_head {
            NetworkMessage::ChainHeadResponse { latest_block_hash, height } => {
                log::info!(
                    "Peer {} has chain at height {} with hash {}",
                    hex::encode(peer.id.to_bytes()),
                    height,
                    crate::types::format_hex(&latest_block_hash)
                );

                // Detect fork at same height with different hash
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
                    // Trigger fork resolution by requesting fork blocks
                    return self
                        .resolve_fork_blocks(peer, &mut stream, local_chain_state, height)
                        .await;
                }

                if height < local_chain_state.latest_block_index {
                    // Peer is behind — try resolving fork in case our chain is stale
                    let peer_state = ChainState {
                        latest_block_hash,
                        latest_block_index: height,
                        accounts_root_hash: [0u8; 32],
                        total_supply: 0,
                    };
                    self.resolve_fork(&peer_state, local_chain_state, peer).await?;
                    return Err(SyncError::SynchronizationError(
                        "Peer is not ahead of local chain".to_string(),
                    ));
                }

                if height == local_chain_state.latest_block_index {
                    return Err(SyncError::SynchronizationError(
                        "Peer is not ahead of local chain".to_string(),
                    ));
                }

                if height > local_chain_state.latest_block_index + 1 {
                    Self::send_message(
                        &mut stream,
                        NetworkMessage::GetBlocks {
                            from_height: local_chain_state.latest_block_index + 1,
                            to_height: height,
                        },
                    )
                    .await?;
                    let blocks_response = Self::receive_message(&mut stream).await?;
                    match blocks_response {
                        NetworkMessage::BlocksResponse { blocks } => {
                            if blocks.is_empty() {
                                return Err(SyncError::SynchronizationError(
                                    "Empty blocks response".to_string(),
                                ));
                            }
                            // Detect fork: if first block doesn't chain to our head,
                            // the peer diverged before our current height.
                            if blocks[0].prev_hash != local_chain_state.latest_block_hash {
                                log::warn!(
                                    "Fork detected: received block #{} prev_hash \
                                     doesn't match local head",
                                    blocks[0].index
                                );
                                return self
                                    .resolve_fork_blocks(
                                        peer,
                                        &mut stream,
                                        local_chain_state,
                                        height,
                                    )
                                    .await;
                            }
                            for block in &blocks {
                                let mut cache = self.block_cache.lock().await;
                                if cache.len() < MAX_BLOCK_CACHE_SIZE {
                                    cache.insert(block.hash, block.clone());
                                }
                                drop(cache);
                            }
                            let mut recv = self.received_blocks.lock().await;
                            if recv.len() + blocks.len() <= MAX_RECEIVED_BLOCKS_QUEUE {
                                recv.extend(blocks.clone());
                            } else {
                                log::warn!("Dropped incoming blocks: received_blocks queue full");
                            }
                            return Ok(blocks.last().unwrap().clone());
                        }
                        _ => {
                            return Err(SyncError::SynchronizationError(
                                "Unexpected response for GetBlocks".to_string(),
                            ));
                        }
                    }
                }

                // Peer is ahead by exactly 1: request the latest block
                Self::send_message(
                    &mut stream,
                    NetworkMessage::RequestBlock { hash: latest_block_hash },
                )
                .await?;
                let block_response = Self::receive_message(&mut stream).await?;
                match block_response {
                    NetworkMessage::BlockResponse { block: Some(block) } => {
                        if block.hash != latest_block_hash {
                            return Err(SyncError::SynchronizationError(
                                "Peer returned unexpected block hash".to_string(),
                            ));
                        }
                        let mut cache = self.block_cache.lock().await;
                        cache.insert(block.hash, block.clone());
                        drop(cache);
                        let mut recv = self.received_blocks.lock().await;
                        if recv.len() < MAX_RECEIVED_BLOCKS_QUEUE {
                            recv.push(block.clone());
                        }
                        Ok(block)
                    }
                    NetworkMessage::BlockResponse { block: None } => Err(SyncError::BlockNotFound),
                    _ => Err(SyncError::InvalidMessage),
                }
            }
            _ => Err(SyncError::InvalidMessage),
        }
    }

    async fn discover_peers(&self) -> Result<Vec<Peer>, SyncError> {
        let peers = self.known_peers.read().await;
        Ok(peers.iter().map(|(id, addr)| Peer { id: *id, address: *addr }).collect())
    }

    async fn broadcast_block(&self, block: &Block, peers: &[Peer]) -> Result<(), SyncError> {
        self.cache_block(block.clone()).await;
        let announcement =
            NetworkMessage::NewBlockAnnouncement { block_hash: block.hash, height: block.index };

        for peer in peers {
            if let Ok(mut stream) = self.connect_to_peer(peer.address).await {
                // Perform mutual handshake
                let mut challenge = [0u8; 32];
                rand::RngCore::fill_bytes(&mut rand::rng(), &mut challenge);

                if Self::send_message(
                    &mut stream,
                    NetworkMessage::Handshake { peer_id: self.peer_id, version: 1, challenge },
                )
                .await
                .is_ok()
                {
                    if let Ok(NetworkMessage::HandshakeAck {
                        peer_id: remote_peer_id,
                        signature,
                        challenge: remote_challenge,
                        ..
                    }) = Self::receive_message(&mut stream).await
                    {
                        // Verify their signature
                        let sig_res = Signature::from_slice(&signature);
                        let pk_res =
                            ed25519_dalek::VerifyingKey::from_bytes(&remote_peer_id.to_bytes());

                        if let (Ok(sig), Ok(pk)) = (sig_res, pk_res) {
                            if pk.verify(&challenge, &sig).is_ok() {
                                // Sign their challenge
                                let my_sig = {
                                    let sig_key_guard = self.signing_key.lock().await;
                                    if let Some(sig_key) = sig_key_guard.as_ref() {
                                        sig_key.sign(&remote_challenge).to_vec()
                                    } else {
                                        Vec::new()
                                    }
                                };
                                if !my_sig.is_empty() {
                                    let _ = Self::send_message(
                                        &mut stream,
                                        NetworkMessage::HandshakeVerify { signature: my_sig },
                                    )
                                    .await;
                                    let _ =
                                        Self::send_message(&mut stream, announcement.clone()).await;
                                }
                            }
                        }
                    }
                }
                // Handle the peer's block request with a short timeout
                if let Ok(Ok(NetworkMessage::RequestBlock { hash })) =
                    timeout(Duration::from_secs(2), Self::receive_message(&mut stream)).await
                {
                    Self::handle_block_request_full(
                        &mut stream,
                        hash,
                        &self.block_cache,
                        &self.storage,
                    )
                    .await
                    .ok();
                }
            }
        }

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
        log::info!("Added peer: {}", addr);
        Ok(())
    }

    fn poll_received_blocks(&self) -> Vec<Block> {
        let mut recv = match self.received_blocks.try_lock() {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        std::mem::take(&mut *recv)
    }

    fn stop_listener(&self) {
        let mut guard = self.shutdown_tx.blocking_lock();
        if let Some(tx) = guard.take() {
            let _ = tx.send(true);
            log::info!("P2P listener shutdown signal sent");
        }
    }
}

impl CustomSync {
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

    fn poll_received_blocks(&self) -> Vec<Block> {
        match self {
            SyncWrapper::Noop(n) => n.poll_received_blocks(),
            SyncWrapper::Custom(c) => c.poll_received_blocks(),
        }
    }
}
