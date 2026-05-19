use clap::Subcommand;
use ed25519_dalek::Signer;
use std::io::{self, Write};

use crate::Keystore;

use crate::cli::{parse_pubkey, text_or_json};

#[derive(Subcommand)]
pub enum WalletCommands {
    Create {
        #[arg(short, long)]
        name: Option<String>,
    },
    List,
    Import {
        private_key: String,
        #[arg(short, long)]
        name: Option<String>,
    },
    Export {
        identifier: String,
        #[arg(short, long)]
        password: Option<String>,
    },
    Sign {
        identifier: String,
        message: String,
    },
    Delete {
        identifier: String,
    },
    Rotate {
        identifier: String,
    },
    Show {
        identifier: String,
    },
    Verify {
        identifier: String,
        message: String,
        signature: String,
    },
    ChangePassword {
        identifier: String,
    },
    ExportPublic {
        identifier: String,
    },
    Recover {
        #[arg(long)]
        seed_hex: String,
        #[arg(short, long)]
        name: Option<String>,
    },
}

fn prompt_password(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut stdout = io::stdout();
    write!(stdout, "{}", prompt)?;
    stdout.flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

pub fn handle_wallet(
    action: WalletCommands,
    json: bool,
    _backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let keystore = Keystore::new(None)?;
    match action {
        WalletCommands::Create { name } => {
            let password = prompt_password("Password: ")?;
            let pk = keystore.create_key(&password)?;
            Ok(text_or_json(
                json,
                &format!(
                    "Wallet created ({})\nPublic Key: {}",
                    name.as_deref().unwrap_or("unnamed"),
                    hex::encode(pk.to_bytes())
                ),
                serde_json::json!({"public_key": hex::encode(pk.to_bytes()), "name": name}),
            ))
        }
        WalletCommands::List => {
            let keys = keystore.list_keys()?;
            if keys.is_empty() {
                Ok(text_or_json(json, "No wallets found", serde_json::json!({"wallets": []})))
            } else {
                let list: Vec<String> = keys.iter().map(|k| hex::encode(k.to_bytes())).collect();
                Ok(text_or_json(json, &list.join("\n"), serde_json::json!({"wallets": list})))
            }
        }
        WalletCommands::Import { private_key, name } => {
            let bytes = hex::decode(&private_key)?;
            if bytes.len() != 32 {
                return Err("Private key must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            let password = prompt_password("Encryption password: ")?;
            let pk = keystore.import_key(&arr, &password)?;
            Ok(text_or_json(
                json,
                &format!(
                    "Imported ({}) {}",
                    name.as_deref().unwrap_or("unnamed"),
                    hex::encode(pk.to_bytes())
                ),
                serde_json::json!({"public_key": hex::encode(pk.to_bytes()), "name": name}),
            ))
        }
        WalletCommands::Export { identifier, password } => {
            let pk = parse_pubkey(&identifier)?;
            let pw = password.unwrap_or_else(|| prompt_password("Password: ").unwrap_or_default());
            let sk = keystore.export_key(&pk, &pw)?;
            Ok(text_or_json(
                json,
                &format!("Private key: {}", hex::encode(sk)),
                serde_json::json!({"private_key": hex::encode(sk)}),
            ))
        }
        WalletCommands::Sign { identifier, message } => {
            let pk = parse_pubkey(&identifier)?;
            let password = prompt_password("Password: ")?;
            let sk = keystore.load_key(&pk, &password)?;
            let msg_bytes = if let Some(hex_str) = message.strip_prefix("hex:") {
                hex::decode(hex_str).unwrap_or_else(|_| hex_str.as_bytes().to_vec())
            } else {
                message.as_bytes().to_vec()
            };
            let sig = sk.sign(&msg_bytes);
            Ok(text_or_json(
                json,
                &format!("Signature: {}", hex::encode(sig.to_bytes())),
                serde_json::json!({"signature": hex::encode(sig.to_bytes())}),
            ))
        }
        WalletCommands::Delete { identifier } => {
            let pk = parse_pubkey(&identifier)?;
            keystore.delete_key(&pk)?;
            Ok(text_or_json(
                json,
                &format!("Wallet deleted: {}", hex::encode(pk.to_bytes())),
                serde_json::json!({"deleted": hex::encode(pk.to_bytes())}),
            ))
        }
        WalletCommands::Rotate { identifier } => {
            let pk = parse_pubkey(&identifier)?;
            let password = prompt_password("Current password: ")?;
            let old_sk = keystore.export_key(&pk, &password)?;
            let new_password = prompt_password("New password: ")?;
            let new_pk = keystore.import_key(&old_sk, &new_password)?;
            Ok(text_or_json(
                json,
                &format!(
                    "Key rotated: {}\nNew public key: {}",
                    hex::encode(pk.to_bytes()),
                    hex::encode(new_pk.to_bytes())
                ),
                serde_json::json!({
                    "old_key": hex::encode(pk.to_bytes()),
                    "new_key": hex::encode(new_pk.to_bytes()),
                    "status": "rotated"
                }),
            ))
        }
        WalletCommands::Show { identifier } => {
            let pk = parse_pubkey(&identifier)?;
            let fmt_version = keystore.key_format_version(&pk).unwrap_or(1);
            Ok(text_or_json(
                json,
                &format!("Public Key: {}\nFormat: v{}", hex::encode(pk.to_bytes()), fmt_version,),
                serde_json::json!({
                    "public_key": hex::encode(pk.to_bytes()),
                    "format_version": fmt_version,
                }),
            ))
        }
        WalletCommands::Verify { identifier, message, signature } => {
            let pk = parse_pubkey(&identifier)?;
            let sig_bytes = hex::decode(&signature)?;
            if sig_bytes.len() != 64 {
                return Err("Signature must be 64 bytes hex".into());
            }
            let mut sig_arr = [0u8; 64];
            sig_arr.copy_from_slice(&sig_bytes);
            let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
            let msg_bytes = message.as_bytes().to_vec();
            let valid = pk.verify(&msg_bytes, &sig).is_ok();
            Ok(text_or_json(
                json,
                if valid { "Signature is VALID" } else { "Signature is INVALID" },
                serde_json::json!({"valid": valid}),
            ))
        }
        WalletCommands::ChangePassword { identifier } => {
            let pk = parse_pubkey(&identifier)?;
            let old_password = prompt_password("Current password: ")?;
            let sk_bytes = keystore.export_key(&pk, &old_password)?;
            keystore.delete_key(&pk)?;
            let new_password = prompt_password("New password: ")?;
            let _ = keystore.import_key(&sk_bytes, &new_password)?;
            Ok(text_or_json(
                json,
                "Password changed successfully",
                serde_json::json!({"status": "password_changed"}),
            ))
        }
        WalletCommands::ExportPublic { identifier } => {
            let pk = parse_pubkey(&identifier)?;
            Ok(text_or_json(
                json,
                hex::encode(pk.to_bytes()).as_str(),
                serde_json::json!({"public_key": hex::encode(pk.to_bytes())}),
            ))
        }
        WalletCommands::Recover { seed_hex, name } => {
            let bytes = hex::decode(&seed_hex)?;
            if bytes.len() != 32 {
                return Err("Seed must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            let password = prompt_password("Encryption password: ")?;
            let pk = keystore.import_key(&arr, &password)?;
            Ok(text_or_json(
                json,
                &format!(
                    "Recovered ({}) {}",
                    name.as_deref().unwrap_or("unnamed"),
                    hex::encode(pk.to_bytes())
                ),
                serde_json::json!({"public_key": hex::encode(pk.to_bytes()), "name": name}),
            ))
        }
    }
}
