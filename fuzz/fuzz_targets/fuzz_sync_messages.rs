#![no_main]

use baals::sync::NetworkMessage;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(msg) = bincode::deserialize::<NetworkMessage>(data) {
        match &msg {
            NetworkMessage::Handshake { peer_id, version, challenge } => {
                let _ = peer_id.to_bytes();
                let _ = version;
                let _ = challenge;
            }
            NetworkMessage::HandshakeAck { peer_id, version, signature, challenge } => {
                let _ = peer_id.to_bytes();
                let _ = version;
                let _ = signature.len();
                let _ = challenge;
            }
            NetworkMessage::HandshakeVerify { signature } => {
                let _ = signature.len();
            }
            NetworkMessage::GetChainHead => {}
            NetworkMessage::ChainHeadResponse { latest_block_hash, height } => {
                let _ = latest_block_hash;
                let _ = height;
            }
            NetworkMessage::GetBlocks { from_height, to_height } => {
                let _ = from_height;
                let _ = to_height;
            }
            NetworkMessage::BlocksResponse { blocks } => {
                let _ = blocks.len();
            }
            NetworkMessage::NewBlockAnnouncement { block_hash, height } => {
                let _ = block_hash;
                let _ = height;
            }
            NetworkMessage::ForkResolution { common_height, fork_blocks } => {
                let _ = common_height;
                let _ = fork_blocks.len();
            }
            NetworkMessage::GetForkBlocks { from_height, to_height } => {
                let _ = from_height;
                let _ = to_height;
            }
            NetworkMessage::ForkBlocksResponse { blocks, total_height } => {
                let _ = blocks.len();
                let _ = total_height;
            }
            NetworkMessage::Ping => {}
            NetworkMessage::Pong => {}
            NetworkMessage::PeerList { peers } => {
                let _ = peers.len();
            }
            NetworkMessage::RequestBlock { hash } => {
                let _ = hash;
            }
            NetworkMessage::BlockResponse { block } => {
                if let Some(b) = block {
                    let _ = b.calculate_hash();
                }
            }
        }
    }
});
