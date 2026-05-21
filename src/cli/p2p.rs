use clap::Subcommand;
use std::path::PathBuf;

use crate::cli::text_or_json;

#[derive(Subcommand)]
pub enum P2pCommands {
    Peers {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long, default_value = "8080")]
        port: u16,
    },
    AddPeer {
        address: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long, default_value = "8080")]
        port: u16,
    },
    RemovePeer {
        address: String,
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long, default_value = "8080")]
        port: u16,
    },
    Ping {
        address: String,
    },
    SyncNow {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long, default_value = "8080")]
        port: u16,
    },
}

fn node_api_call(
    method: &str,
    port: u16,
    path: &str,
    body: Option<&str>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let url = format!("http://127.0.0.1:{}{}", port, path);
    let client = std::process::Command::new("curl")
        .args(match (method, body) {
            ("GET", _) => vec!["-s", "-X", "GET", &url],
            ("POST", Some(b)) => {
                vec!["-s", "-X", "POST", "-H", "Content-Type: application/json", "-d", b, &url]
            }
            ("DELETE", _) => vec!["-s", "-X", "DELETE", &url],
            _ => vec!["-s", &url],
        })
        .output();

    match client {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            serde_json::from_str(&text)
                .map_err(|e| format!("Node response was not JSON ({}): {}", e, text).into())
        }
        Ok(out) => Err(format!("curl exited with status {}", out.status).into()),
        Err(_) => Err("Could not reach node — is `baals node start` running on this port?".into()),
    }
}

pub fn handle_p2p(
    action: P2pCommands,
    json: bool,
    _backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        P2pCommands::Peers { port, .. } => {
            let val = node_api_call("GET", port, "/api/v1/peers", None)?;
            Ok(text_or_json(json, &format!("Peers: {}", val), val))
        }
        P2pCommands::AddPeer { address, port, .. } => {
            let body = serde_json::json!({"address": address}).to_string();
            let val = node_api_call("POST", port, "/api/v1/peers", Some(&body))?;
            Ok(text_or_json(json, &format!("Add peer result: {}", val), val))
        }
        P2pCommands::RemovePeer { address, port, .. } => {
            let path = format!("/api/v1/peers/{}", urlencoding(&address));
            let val = node_api_call("DELETE", port, &path, None)?;
            Ok(text_or_json(json, &format!("Remove peer result: {}", val), val))
        }
        P2pCommands::Ping { address } => {
            // Simple TCP ping (no node API needed)
            let socket: Result<std::net::TcpStream, _> = std::net::TcpStream::connect_timeout(
                &address.parse()?,
                std::time::Duration::from_secs(2),
            );
            match socket {
                Ok(_) => Ok(text_or_json(
                    json,
                    &format!("Reachable: {}", address),
                    serde_json::json!({"address": address, "reachable": true}),
                )),
                Err(e) => Err(format!("Unreachable {}: {}", address, e).into()),
            }
        }
        P2pCommands::SyncNow { port, .. } => {
            let val = node_api_call("POST", port, "/api/v1/sync/trigger", Some("{}"))?;
            Ok(text_or_json(json, &format!("Sync triggered: {}", val), val))
        }
    }
}

fn urlencoding(s: &str) -> String {
    s.replace(':', "%3A").replace('/', "%2F")
}
