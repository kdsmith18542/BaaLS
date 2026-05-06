use async_trait::async_trait;
use bincode;
use hex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
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
}

impl CustomSync {
    pub fn new(peer_id: PublicKey, listen_addr: SocketAddr) -> Self {
        Self {
            peer_id,
            known_peers: Arc::new(Mutex::new(HashMap::new())),
            block_cache: Arc::new(Mutex::new(HashMap::new())),
            listen_addr,
            is_running: Arc::new(Mutex::new(false)),
        }
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

        loop {
            let (socket, addr) = listener
                .accept()
                .await
                .map_err(|e| SyncError::NetworkError(e.to_string()))?;

            let peer_id = self.peer_id;
            let peers = Arc::clone(&self.known_peers);
            let block_cache = Arc::clone(&self.block_cache);

            tokio::spawn(async move {
                if let Err(e) =
                    Self::handle_connection(socket, addr, peer_id, peers, block_cache).await
                {
                    eprintln!("Connection error: {}", e);
                }
            });
        }
    }

    async fn handle_connection(
        mut socket: TcpStream,
        addr: SocketAddr,
        peer_id: PublicKey,
        peers: Arc<Mutex<HashMap<PublicKey, SocketAddr>>>,
        block_cache: Arc<Mutex<HashMap<[u8; 32], Block>>>,
    ) -> Result<(), SyncError> {
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

    async fn send_message(
        stream: &mut TcpStream,
        message: NetworkMessage,
    ) -> Result<(), SyncError> {
        const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;
        let serialized = bincode::serialize(&message)
            .map_err(|e| SyncError::SerializationError(e.to_string()))?;
        if serialized.len() > MAX_MESSAGE_SIZE as usize {
            return Err(SyncError::InvalidMessage);
        }
        let frame = MessageFrame::new(message)?;
        let bytes = frame.to_bytes()?;
        tokio::io::AsyncWriteExt::write_all(stream, &bytes)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        Ok(())
    }

    async fn receive_message(stream: &mut TcpStream) -> Result<NetworkMessage, SyncError> {
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
    async fn send_peer_list(&self, stream: &mut TcpStream) -> Result<(), SyncError> {
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
    async fn handle_block_request(
        stream: &mut TcpStream,
        message: NetworkMessage,
        block_cache: &Arc<Mutex<HashMap<[u8; 32], Block>>>,
    ) -> Result<(), SyncError> {
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
