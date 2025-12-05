use thiserror::Error;
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use serde::{Deserialize, Serialize};
use bincode;
use hex;

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
    Handshake { peer_id: PublicKey, version: u32 },
    HandshakeAck { peer_id: PublicKey, version: u32 },
    
    // Sync protocol messages
    GetChainHead,
    ChainHeadResponse { latest_block_hash: [u8; 32], height: u64 },
    GetBlocks { from_height: u64, to_height: u64 },
    BlocksResponse { blocks: Vec<Block> },
    NewBlockAnnouncement { block_hash: [u8; 32], height: u64 },
    
    // Keep-alive
    Ping,
    Pong,
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
    async fn sync_with_peer(&self, peer: &Peer, local_chain_state: &ChainState) -> Result<Block, SyncError>;
    
    /// Discovers new peers in the network.
    async fn discover_peers(&self) -> Result<Vec<Peer>, SyncError>;
    
    /// Broadcasts a new block to known peers.
    async fn broadcast_block(&self, block: &Block, peers: &[Peer]) -> Result<(), SyncError>;
}

/// Minimal custom P2P sync implementation
pub struct CustomSync {
    peer_id: PublicKey,
    known_peers: Arc<Mutex<HashMap<PublicKey, SocketAddr>>>,
    listen_addr: SocketAddr,
    is_running: Arc<Mutex<bool>>,
}

impl CustomSync {
    pub fn new(peer_id: PublicKey, listen_addr: SocketAddr) -> Self {
        Self {
            peer_id,
            known_peers: Arc::new(Mutex::new(HashMap::new())),
            listen_addr,
            is_running: Arc::new(Mutex::new(false)),
        }
    }
    
    pub async fn add_peer(&self, peer: Peer) {
        let mut peers = self.known_peers.lock().await;
        peers.insert(peer.id, peer.address);
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
            let (socket, addr) = listener.accept().await
                .map_err(|e| SyncError::NetworkError(e.to_string()))?;
            
            let peer_id = self.peer_id;
            let peers = Arc::clone(&self.known_peers);
            
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(socket, addr, peer_id, peers).await {
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
    ) -> Result<(), SyncError> {
        // Simple handshake
        let handshake = MessageFrame::new(NetworkMessage::Handshake {
            peer_id,
            version: 1,
        })?;
        
        let handshake_bytes = handshake.to_bytes()?;
        tokio::io::AsyncWriteExt::write_all(&mut socket, &handshake_bytes).await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        
        // Read response
        let mut buffer = [0u8; 1024];
        let n = tokio::io::AsyncReadExt::read(&mut socket, &mut buffer).await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        
        if n == 0 {
            return Err(SyncError::NetworkError("Connection closed".to_string()));
        }
        
        let frame = MessageFrame::from_bytes(&buffer[..n])?;
        match frame.message {
            NetworkMessage::HandshakeAck { peer_id: remote_peer_id, version } => {
                if version != 1 {
                    return Err(SyncError::NetworkError("Version mismatch".to_string()));
                }
                
                // Add to known peers
                let mut peers_guard = peers.lock().await;
                peers_guard.insert(remote_peer_id, addr);
                println!("New peer connected: {} at {}", hex::encode(remote_peer_id.to_bytes()), addr);
            }
            _ => return Err(SyncError::InvalidMessage),
        }
        
        Ok(())
    }
    
    async fn send_message(stream: &mut TcpStream, message: NetworkMessage) -> Result<(), SyncError> {
        let frame = MessageFrame::new(message)?;
        let bytes = frame.to_bytes()?;
        tokio::io::AsyncWriteExt::write_all(stream, &bytes).await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        Ok(())
    }
    
    async fn receive_message(stream: &mut TcpStream) -> Result<NetworkMessage, SyncError> {
        let mut length_buffer = [0u8; 4];
        tokio::io::AsyncReadExt::read_exact(stream, &mut length_buffer).await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        
        let length = u32::from_le_bytes(length_buffer);
        let mut message_buffer = vec![0u8; length as usize];
        tokio::io::AsyncReadExt::read_exact(stream, &mut message_buffer).await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        
        let frame = MessageFrame::from_bytes(&message_buffer)?;
        Ok(frame.message)
    }
}

#[async_trait]
impl SyncLayer for CustomSync {
    async fn sync_with_peer(&self, peer: &Peer, _local_chain_state: &ChainState) -> Result<Block, SyncError> {
        let mut stream = timeout(
            Duration::from_secs(5),
            TcpStream::connect(peer.address)
        ).await
            .map_err(|_| SyncError::ConnectionTimeout)?
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        
        // Perform handshake
        Self::send_message(&mut stream, NetworkMessage::Handshake {
            peer_id: self.peer_id,
            version: 1,
        }).await?;
        
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
            NetworkMessage::ChainHeadResponse { latest_block_hash, height } => {
                println!("Peer {} has chain at height {} with hash {}", 
                    hex::encode(peer.id.to_bytes()), height, crate::types::format_hex(&latest_block_hash));
                
                // For now, return a dummy block. In real implementation,
                // we would fetch the actual block and validate it.
                Err(SyncError::SynchronizationError("Sync not fully implemented".to_string()))
            }
            _ => Err(SyncError::InvalidMessage),
        }
    }
    
    async fn discover_peers(&self) -> Result<Vec<Peer>, SyncError> {
        let peers = self.known_peers.lock().await;
        Ok(peers.iter()
            .map(|(id, addr)| Peer { id: *id, address: *addr })
            .collect())
    }
    
    async fn broadcast_block(&self, block: &Block, peers: &[Peer]) -> Result<(), SyncError> {
        let announcement = NetworkMessage::NewBlockAnnouncement {
            block_hash: block.hash,
            height: block.index,
        };
        
        for peer in peers {
            if let Ok(stream) = timeout(
                Duration::from_secs(2),
                TcpStream::connect(peer.address)
            ).await {
                if let Ok(stream) = stream {
                    let mut stream = stream;
                    if let Err(e) = Self::send_message(&mut stream, announcement.clone()).await {
                        eprintln!("Failed to broadcast to {}: {}", hex::encode(peer.id.to_bytes()), e);
                    }
                }
            }
        }
        
        Ok(())
    }
}

/// No-operation implementation for testing
#[derive(Debug, Clone)]
pub struct NoopSync;

#[async_trait]
impl SyncLayer for NoopSync {
    async fn sync_with_peer(&self, _peer: &Peer, _local_chain_state: &ChainState) -> Result<Block, SyncError> {
        Err(SyncError::SynchronizationError("No-op sync does not perform actual synchronization".to_string()))
    }
    
    async fn discover_peers(&self) -> Result<Vec<Peer>, SyncError> {
        Ok(vec![])
    }
    
    async fn broadcast_block(&self, _block: &Block, _peers: &[Peer]) -> Result<(), SyncError> {
        Ok(())
    }
} 