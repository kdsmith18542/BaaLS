use clap::Subcommand;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::cli::text_or_json;
use crate::PublicKey;

#[derive(Subcommand)]
pub enum AdminCommands {
    RotateConsensusKey {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long)]
        new_key_path: Option<PathBuf>,
    },
    ExportNodeId {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    TlsGenerate {
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value = "baals-node")]
        cn: String,
        #[arg(long, default_value = "BaaLS")]
        org: String,
        #[arg(long, default_value_t = 365)]
        days: u32,
        #[arg(long, default_values = &["DNS:localhost", "IP:127.0.0.1"])]
        san: Vec<String>,
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
        AdminCommands::RotateConsensusKey { data_dir, new_key_path } => {
            let new_key_path = new_key_path.unwrap_or_else(|| data_dir.join("consensus_rotated.key"));
            let mut sk_bytes = [0u8; 32];
            rand::rng().fill_bytes(&mut sk_bytes);
            let new_key = ed25519_dalek::SigningKey::from_bytes(&sk_bytes);
            let new_pk = PublicKey::from(new_key.verifying_key());
            std::fs::create_dir_all(new_key_path.parent().unwrap_or(&data_dir))?;
            std::fs::write(&new_key_path, &sk_bytes)?;
            Ok(text_or_json(
                json,
                &format!(
                    "New consensus key generated.\nPublic key: {}\nSaved to: {}\n\n\
                     To activate:\n1. Stop the node\n2. Replace {} with the new key\n\
                     3. If using authorized signers, add the new public key\n4. Restart the node",
                    hex::encode(new_pk.to_bytes()),
                    new_key_path.display(),
                    new_key_path.display()
                ),
                serde_json::json!({
                    "status": "key_generated",
                    "public_key": hex::encode(new_pk.to_bytes()),
                    "key_file": new_key_path.to_string_lossy(),
                    "rotation_instructions": "Stop node, update key file, add to authorized signers if needed, restart"
                }),
            ))
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
        AdminCommands::TlsGenerate { output, cn, org, days, san } => {
            use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
            use std::time::SystemTime;

            let mut params = CertificateParams::new(san.clone())?;

            let mut dn = DistinguishedName::new();
            dn.push(DnType::CommonName, &cn);
            dn.push(DnType::OrganizationName, &org);
            params.distinguished_name = dn;

            let now = SystemTime::now();
            params.not_before = now.into();
            params.not_after = (now + std::time::Duration::from_secs(days as u64 * 86400)).into();

            let key_pair = KeyPair::generate()?;
            let cert = params.self_signed(&key_pair)?;

            let cert_pem = cert.pem();
            let key_pem = key_pair.serialize_pem();

            std::fs::create_dir_all(&output)?;

            let cert_path = output.join("server.crt");
            std::fs::write(&cert_path, &cert_pem)?;

            let key_path = output.join("server.key");
            std::fs::write(&key_path, &key_pem)?;

            Ok(text_or_json(
                json,
                &format!(
                    "TLS certificate generated.\nCertificate: {}\nPrivate key: {}\n\n\
                     Add to config:\n  [network]\n  tls_enabled = true\n  tls_cert_path = \"{}\"\n  tls_key_path = \"{}\"",
                    cert_path.display(),
                    key_path.display(),
                    cert_path.to_string_lossy().replace("\\", "/"),
                    key_path.to_string_lossy().replace("\\", "/")
                ),
                serde_json::json!({
                    "status": "tls_certificates_generated",
                    "cert_path": cert_path.to_string_lossy(),
                    "key_path": key_path.to_string_lossy(),
                    "subject": cn,
                    "organization": org,
                    "valid_days": days,
                }),
            ))
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
