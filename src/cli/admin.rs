use clap::Subcommand;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::cli::text_or_json;

#[derive(Subcommand)]
pub enum AdminCommands {
    RotateConsensusKey {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    ExportNodeId {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    TlsGenerate {
        #[arg(short, long)]
        output: PathBuf,
    },
    TlsFingerprint {
        #[arg(short, long)]
        cert_path: PathBuf,
    },
    TokenGenerate {
        #[arg(short, long)]
        length: Option<usize>,
    },
}

pub fn handle_admin(
    action: AdminCommands,
    json: bool,
    _backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        AdminCommands::RotateConsensusKey { .. } => {
            let msg = "admin rotate-consensus-key is not yet implemented as an atomic workflow";
            let _ = text_or_json(
                json,
                msg,
                serde_json::json!({
                    "error": "not_implemented",
                    "command": "admin rotate-consensus-key",
                    "message": msg,
                }),
            );
            Err(msg.into())
        }
        AdminCommands::ExportNodeId { data_dir } => {
            let mut hasher = Sha256::new();
            Digest::update(&mut hasher, b"baals-node");
            let node_id = hex::encode(hasher.finalize());
            let node_id_file = data_dir.join("node_id");
            std::fs::write(&node_id_file, &node_id)?;
            Ok(text_or_json(
                json,
                &format!("Node ID: {}\nWritten to: {}", node_id, node_id_file.display()),
                serde_json::json!({"node_id": node_id, "file": node_id_file.to_string_lossy()}),
            ))
        }
        AdminCommands::TlsGenerate { output } => {
            let msg = format!(
                "admin tls-generate is not implemented in-process. Use docs/TLS_GUIDE.md for external generation (target output: {}).",
                output.display()
            );
            let _ = text_or_json(
                json,
                &msg,
                serde_json::json!({
                    "error": "not_implemented",
                    "command": "admin tls-generate",
                    "output": output.to_string_lossy(),
                    "message": msg,
                }),
            );
            Err(msg.into())
        }
        AdminCommands::TlsFingerprint { cert_path } => {
            let cert_data = std::fs::read(&cert_path)?;
            let mut hasher = Sha256::new();
            Digest::update(&mut hasher, &cert_data);
            let fingerprint = hex::encode(hasher.finalize());
            Ok(text_or_json(
                json,
                &format!("SHA256 Fingerprint: {}", fingerprint),
                serde_json::json!({"path": cert_path.to_string_lossy(), "fingerprint": fingerprint}),
            ))
        }
        AdminCommands::TokenGenerate { length } => {
            let len = length.unwrap_or(32);
            let mut token_bytes = vec![0u8; len];
            rand::rng().fill_bytes(&mut token_bytes);
            let token = hex::encode(&token_bytes);
            Ok(text_or_json(
                json,
                &format!("Generated {} byte token:\n{}", len, token),
                serde_json::json!({"length": len, "token": token}),
            ))
        }
    }
}
