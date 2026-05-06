use async_trait::async_trait;
use bincode;
use hex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

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
    },
    HandshakeAck {
        peer_id: PublicKey,
        version: u32,
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
        Ok(MessageFrame {
            length: message_bytes.len() as u32,
            message,
        })
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
}

/// Minimal custom P2P sync implementation
pub struct CustomSync {
    peer_id: PublicKey,
    known_peers: Arc<Mutex<HashMap<PublicKey, SocketAddr>>>,
    block_cache: Arc<Mutex<HashMap<[u8; 32], Block>>>,
    listen_addr: SocketAddr,
    is_running: Arc<Mutex<bool>>,
    tls_config: Option<Arc<TlsConfig>>,
}

/// TLS configuration for P2P connections
pub struct TlsConfig {
    pub server_config: tokio_rustls::rustls::ServerConfig,
    pub client_config: tokio_rustls::rustls::ClientConfig,
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
        }
    }
}

impl TlsConfig {
    /// Load TLS config from certificate and key files
    pub fn load(
        cert_path: &str,
        key_path: &str,
        _ca_cert_path: Option<&str>,
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
            .map(|k| rustls::pki_types::PrivateKeyDer::Pkcs8(k))
            .collect();
        if keys.is_empty() {
            return Err(SyncError::NetworkError(
                "No private keys found in key file".to_string(),
            ));
        }

        // Build server config
        let server_config = tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain.clone(), keys[0].clone_key())
            .map_err(|e| SyncError::NetworkError(format!("Failed to build server TLS config: {}", e)))?;

        // Build client config — accept all certs for P2P mode
        let client_config = tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification::new()))
            .with_no_client_auth();

        Ok(TlsConfig {
            server_config,
            client_config,
        })
    }

    /// Generate a self-signed TLS config for development
    pub fn generate_self_signed() -> Result<Self, SyncError> {
        use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

        let mut params = CertificateParams::new(Vec::new())
            .map_err(|e| SyncError::NetworkError(format!("rcgen error: {}", e)))?;
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, "baals-node");
        params.distinguished_name.push(DnType::OrganizationName, "BaaLS");

        let key_pair = KeyPair::generate().map_err(|e| {
            SyncError::NetworkError(format!("Failed to generate key pair: {}", e))
        })?;
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

        let client_config = tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification::new()))
            .with_no_client_auth();

        Ok(TlsConfig {
            server_config,
            client_config,
        })
    }
}

/// No-op certificate verifier for P2P mode (accepts all certs)
#[derive(Debug)]
struct NoCertificateVerification(rustls::crypto::CryptoProvider);

impl NoCertificateVerification {
    fn new() -> Self {
        Self(rustls::crypto::aws_lc_rs::default_provider())
    }
}

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer,
        _intermediates: &[rustls::pki_types::CertificateDer],
        _server_name: &rustls::pki_types::ServerName,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
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
            &self.0.signature_verification_algorithms,
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
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl CustomSync {
    pub fn new(peer_id: PublicKey, listen_addr: SocketAddr) -> Self {
        Self {
            peer_id,
            known_peers: Arc::new(Mutex::new(HashMap::new())),
            block_cache: Arc::new(Mutex::new(HashMap::new())),
            listen_addr,
            is_running: Arc::new(Mutex::new(false)),
            tls_config: None,
        }
    }

    pub fn with_tls(mut self, tls_config: TlsConfig) -> Self {
        self.tls_config = Some(Arc::new(tls_config));
        self
    }

    pub async fn cache_block(&self, block: Block) {
        let mut cache = self.block_cache.lock().await;
        cache.insert(block.hash, block);
    }

    pub async fn add_peer(&self, peer: Peer) {
        let mut peers = self.known_peers.lock().await;
        peers.insert(peer.id, peer.address);
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

        let mut stream = timeout(Duration::from_secs(5), TcpStream::connect(peer.address))
            .await
            .map_err(|_| SyncError::ConnectionTimeout)?
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        Self::send_message(
            &mut stream,
            NetworkMessage::Handshake {
                peer_id: self.peer_id,
                version: 1,
            },
        )
        .await?;

        let response = Self::receive_message(&mut stream).await?;
        match response {
            NetworkMessage::HandshakeAck { .. } => {}
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
            NetworkMessage::ForkBlocksResponse {
                blocks,
                total_height: _,
            } => {
                for block in &blocks {
                    self.cache_block(block.clone()).await;
                }
                if let Some(last) = blocks.last() {
                    Ok(last.index)
                } else {
                    Ok(local_chain_state.latest_block_index)
                }
            }
            _ => Err(SyncError::SynchronizationError(
                "Failed to receive fork blocks".to_string(),
            )),
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

        let listener = TcpListener::bind(self.listen_addr)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        println!("P2P server listening on {}", self.listen_addr);

        let tls_acceptor: Option<tokio_rustls::TlsAcceptor> = self.tls_config.as_ref().map(|tc| {
            tokio_rustls::TlsAcceptor::from(Arc::new(tc.server_config.clone()))
        });

        loop {
            let (socket, addr) = listener
                .accept()
                .await
                .map_err(|e| SyncError::NetworkError(e.to_string()))?;

            let peer_id = self.peer_id;
            let peers = Arc::clone(&self.known_peers);
            let block_cache = Arc::clone(&self.block_cache);
            let tls_acceptor = tls_acceptor.clone();

            tokio::spawn(async move {
                if let Some(acceptor) = tls_acceptor {
                    match acceptor.accept(socket).await {
                        Ok(tls_stream) => {
                            if let Err(e) = Self::handle_connection(
                                tls_stream, addr, peer_id, peers, block_cache,
                            ).await {
                                eprintln!("TLS connection error: {}", e);
                            }
                        }
                        Err(e) => {
                            eprintln!("TLS handshake error from {}: {}", addr, e);
                        }
                    }
                } else {
                    if let Err(e) = Self::handle_connection(
                        socket, addr, peer_id, peers, block_cache,
                    ).await {
                        eprintln!("Connection error: {}", e);
                    }
                }
            });
        }
    }

    async fn handle_connection<S>(
        mut socket: S,
        addr: SocketAddr,
        peer_id: PublicKey,
        peers: Arc<Mutex<HashMap<PublicKey, SocketAddr>>>,
        block_cache: Arc<Mutex<HashMap<[u8; 32], Block>>>,
    ) -> Result<(), SyncError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        // Expect inbound handshake from peer, then ack.
        let inbound = Self::receive_message(&mut socket).await?;
        match inbound {
            NetworkMessage::Handshake {
                peer_id: remote_peer_id,
                version,
            } => {
                if version != 1 {
                    return Err(SyncError::NetworkError("Version mismatch".to_string()));
                }
                // Add to known peers
                let mut peers_guard = peers.lock().await;
                peers_guard.insert(remote_peer_id, addr);
                drop(peers_guard);

                Self::send_message(
                    &mut socket,
                    NetworkMessage::HandshakeAck {
                        peer_id,
                        version: 1,
                    },
                )
                .await?;
                println!(
                    "New peer connected: {} at {}",
                    hex::encode(remote_peer_id.to_bytes()),
                    addr
                );
            }
            _ => return Err(SyncError::InvalidMessage),
        }
        // After handshake, enter message loop
        loop {
            let msg = match Self::receive_message(&mut socket).await {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Error receiving message: {}", e);
                    break;
                }
            };
            match msg {
                NetworkMessage::PeerList { peers: peer_list } => {
                    // Update known peers
                    let mut peers_guard = peers.lock().await;
                    for (id, addr_str) in peer_list {
                        if let Ok(addr) = addr_str.parse() {
                            peers_guard.insert(id, addr);
                        }
                    }
                }
                NetworkMessage::RequestBlock { hash: _ } => {
                    Self::handle_block_request(&mut socket, msg, &block_cache).await?;
                }
                NetworkMessage::BlockResponse { block } => {
                    Self::handle_block_response(block, &block_cache).await;
                }
                NetworkMessage::GetChainHead => {
                    let (latest_hash, latest_height) =
                        Self::chain_head_from_cache(&block_cache).await;
                    Self::send_message(
                        &mut socket,
                        NetworkMessage::ChainHeadResponse {
                            latest_block_hash: latest_hash,
                            height: latest_height,
                        },
                    )
                    .await?;
                }
                NetworkMessage::GetBlocks {
                    from_height,
                    to_height,
                } => {
                    let blocks = Self::blocks_in_range(&block_cache, from_height, to_height).await;
                    Self::send_message(&mut socket, NetworkMessage::BlocksResponse { blocks })
                        .await?;
                }
                NetworkMessage::GetForkBlocks {
                    from_height,
                    to_height,
                } => {
                    let blocks = Self::blocks_in_range(&block_cache, from_height, to_height).await;
                    Self::send_message(
                        &mut socket,
                        NetworkMessage::ForkBlocksResponse {
                            blocks,
                            total_height: Self::chain_head_from_cache(&block_cache).await.1,
                        },
                    )
                    .await?;
                }
                NetworkMessage::ForkResolution {
                    common_height: _,
                    fork_blocks,
                } => {
                    for block in fork_blocks {
                        let mut cache = block_cache.lock().await;
                        cache.insert(block.hash, block);
                    }
                }
                _ => {
                    // Handle other messages as needed
                }
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
        let frame = MessageFrame {
            length: serialized.len() as u32,
            message,
        };
        let frame_bytes = frame.to_bytes()?;
        stream
            .write_all(&frame_bytes)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        Ok(())
    }

    async fn receive_message<S>(stream: &mut S) -> Result<NetworkMessage, SyncError>
    where
        S: AsyncRead + Unpin,
    {
        let mut length_buffer = [0u8; 4];
        tokio::io::AsyncReadExt::read_exact(stream, &mut length_buffer)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        let length = u32::from_le_bytes(length_buffer);
        const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024; // 16MB
        if length > MAX_MESSAGE_SIZE {
            return Err(SyncError::InvalidMessage);
        }
        let mut message_buffer = vec![0u8; length as usize];
        tokio::io::AsyncReadExt::read_exact(stream, &mut message_buffer)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        let message: NetworkMessage = bincode::deserialize(&message_buffer)
            .map_err(|e| SyncError::SerializationError(e.to_string()))?;
        Ok(message)
    }

    // Send peer list to a peer
    #[allow(dead_code)]
    async fn send_peer_list<S>(&self, stream: &mut S) -> Result<(), SyncError>
    where
        S: AsyncWrite + Unpin + Send,
    {
        let peers = self.known_peers.lock().await;
        let peer_list: Vec<(PublicKey, String)> = peers
            .iter()
            .map(|(id, addr)| (*id, addr.to_string()))
            .collect();
        Self::send_message(stream, NetworkMessage::PeerList { peers: peer_list }).await
    }

    // Handle incoming peer list
    #[allow(dead_code)]
    async fn handle_peer_list(&self, peers: Vec<(PublicKey, String)>) {
        let mut known = self.known_peers.lock().await;
        for (id, addr) in peers {
            if !known.contains_key(&id) {
                if let Ok(sock_addr) = addr.parse() {
                    known.insert(id, sock_addr);
                }
            }
        }
    }

    // Handle block request
    async fn handle_block_request<S>(
        stream: &mut S,
        message: NetworkMessage,
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
    ) -> Result<(), SyncError>
    where
        S: AsyncWrite + Unpin + Send,
    {
        let hash = match message {
            NetworkMessage::RequestBlock { hash } => hash,
            _ => return Err(SyncError::InvalidMessage),
        };
        let block = {
            let cache = block_cache.lock().await;
            cache.get(&hash).cloned()
        };
        Self::send_message(stream, NetworkMessage::BlockResponse { block }).await
    }

    // Handle block response
    async fn handle_block_response(
        block: Option<crate::types::Block>,
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
    ) {
        if let Some(block) = block {
            if let Ok(calculated_hash) = block.calculate_hash() {
                if calculated_hash == block.hash {
                    let mut cache = block_cache.lock().await;
                    cache.insert(block.hash, block);
                }
            }
        }
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
        from_height: u64,
        to_height: u64,
    ) -> Vec<Block> {
        let cache = block_cache.lock().await;
        let mut blocks: Vec<Block> = cache
            .values()
            .filter(|block| block.index >= from_height && block.index <= to_height)
            .cloned()
            .collect();
        blocks.sort_by_key(|block| block.index);
        blocks
    }
}

#[async_trait]
impl SyncLayer for CustomSync {
    async fn sync_with_peer(
        &self,
        peer: &Peer,
        local_chain_state: &ChainState,
    ) -> Result<Block, SyncError> {
        let mut stream = timeout(Duration::from_secs(5), TcpStream::connect(peer.address))
            .await
            .map_err(|_| SyncError::ConnectionTimeout)?
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        // Perform handshake
        Self::send_message(
            &mut stream,
            NetworkMessage::Handshake {
                peer_id: self.peer_id,
                version: 1,
            },
        )
        .await?;

        let response = Self::receive_message(&mut stream).await?;
        match response {
            NetworkMessage::HandshakeAck { .. } => {
                // Handshake successful, proceed with sync
            }
            _ => return Err(SyncError::AuthenticationFailed),
        }

        // Request chain head
        Self::send_message(&mut stream, NetworkMessage::GetChainHead).await?;
        let chain_head = Self::receive_message(&mut stream).await?;

        match chain_head {
            NetworkMessage::ChainHeadResponse {
                latest_block_hash,
                height,
            } => {
                println!(
                    "Peer {} has chain at height {} with hash {}",
                    hex::encode(peer.id.to_bytes()),
                    height,
                    crate::types::format_hex(&latest_block_hash)
                );

                if self.detect_fork(
                    &latest_block_hash,
                    height,
                    local_chain_state.latest_block_index,
                    &local_chain_state.latest_block_hash,
                ) {
                    println!(
                        "FORK DETECTED: peer at height {} has different hash {} vs local {}",
                        height,
                        crate::types::format_hex(&latest_block_hash),
                        crate::types::format_hex(&local_chain_state.latest_block_hash)
                    );
                }

                if height <= local_chain_state.latest_block_index {
                    return Err(SyncError::SynchronizationError(
                        "Peer is not ahead of local chain".to_string(),
                    ));
                }

                Self::send_message(
                    &mut stream,
                    NetworkMessage::RequestBlock {
                        hash: latest_block_hash,
                    },
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
                        Self::handle_block_response(Some(block.clone()), &self.block_cache).await;
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
        let peers = self.known_peers.lock().await;
        Ok(peers
            .iter()
            .map(|(id, addr)| Peer {
                id: *id,
                address: *addr,
            })
            .collect())
    }

    async fn broadcast_block(&self, block: &Block, peers: &[Peer]) -> Result<(), SyncError> {
        self.cache_block(block.clone()).await;
        let announcement = NetworkMessage::NewBlockAnnouncement {
            block_hash: block.hash,
            height: block.index,
        };

        for peer in peers {
            if let Ok(stream) =
                timeout(Duration::from_secs(2), TcpStream::connect(peer.address)).await
            {
                if let Ok(stream) = stream {
                    let mut stream = stream;
                    if let Err(e) = Self::send_message(&mut stream, announcement.clone()).await {
                        eprintln!(
                            "Failed to broadcast to {}: {}",
                            hex::encode(peer.id.to_bytes()),
                            e
                        );
                    }
                }
            }
        }

        Ok(())
    }

    fn peer_count(&self) -> usize {
        match self.known_peers.try_lock() {
            Ok(peers) => peers.len(),
            Err(_) => 0,
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
