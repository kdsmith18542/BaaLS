use clap::Subcommand;
use std::path::PathBuf;

use crate::cli::text_or_json;

#[derive(Subcommand)]
pub enum P2pCommands {
    Peers {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    AddPeer {
        address: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    RemovePeer {
        address: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    Ping {
        address: String,
    },
    SyncNow {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
}

pub fn handle_p2p(
    action: P2pCommands,
    json: bool,
    _backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let (command, details) = match action {
        P2pCommands::Peers { data_dir } => ("peers", serde_json::json!({ "data_dir": data_dir })),
        P2pCommands::AddPeer { address, data_dir } => {
            ("add-peer", serde_json::json!({ "address": address, "data_dir": data_dir }))
        }
        P2pCommands::RemovePeer { address, data_dir } => {
            ("remove-peer", serde_json::json!({ "address": address, "data_dir": data_dir }))
        }
        P2pCommands::Ping { address } => ("ping", serde_json::json!({ "address": address })),
        P2pCommands::SyncNow { data_dir } => {
            ("sync-now", serde_json::json!({ "data_dir": data_dir }))
        }
    };
    let message = format!(
        "p2p {} is not yet wired to live runtime/sync state. \
Use `node start` plus runtime-backed sync APIs for operational P2P actions.",
        command
    );
    let _ = text_or_json(
        json,
        &message,
        serde_json::json!({
            "error": "not_implemented",
            "command": command,
            "details": details,
            "message": message,
        }),
    );
    Err(message.into())
}
