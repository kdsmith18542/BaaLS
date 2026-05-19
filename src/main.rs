use clap::Parser;
use log::error;

use baals::{
    cli::{
        self, admin, contract, db, dev, json_err, node, p2p, parse_pubkey, proof, query,
        text_or_json, tx, wallet, Cli, Commands, KeyCommands,
    },
    config::{setup_logging, Config},
};

fn main() {
    let cli = Cli::parse();
    let log_level = if cli.verbose { "debug" } else { "info" };
    let config = Config::load(None).unwrap_or_default();
    let _ = setup_logging(&config, log_level);

    let result = match cli.command {
        Commands::Node { action } => node::handle_node(action, cli.json, &cli.storage_backend),
        Commands::Wallet { action } => {
            wallet::handle_wallet(action, cli.json, &cli.storage_backend)
        }
        Commands::Tx { action } => tx::handle_tx(action, cli.json, &cli.storage_backend),
        Commands::Query { action } => query::handle_query(action, cli.json, &cli.storage_backend),
        Commands::Dev { action } => dev::handle_dev(action, cli.json, &cli.storage_backend),
        Commands::Db { command } => db::handle_db(command, cli.json, &cli.storage_backend),
        Commands::Key { action } => handle_key(action, cli.json),
        Commands::Proof { action } => proof::handle_proof(action, cli.json, &cli.storage_backend),
        Commands::P2p { action } => p2p::handle_p2p(action, cli.json, &cli.storage_backend),
        Commands::Contract { action } => {
            contract::handle_contract(action, cli.json, &cli.storage_backend)
        }
        Commands::Admin { action } => admin::handle_admin(action, cli.json, &cli.storage_backend),
        Commands::Api { action } => handle_api(action, cli.json),
        Commands::Doctor => handle_doctor(cli.json),
    };

    match result {
        Ok(output) => println!("{}", output),
        Err(e) => {
            error!("{}", e);
            if cli.json {
                println!("{}", json_err(&e.to_string()));
            } else {
                eprintln!("Error: {}", e);
            }
            std::process::exit(1);
        }
    }
}

fn handle_key(action: KeyCommands, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        KeyCommands::Generate => {
            use baals::{AnyStorage, PoAConsensus, PublicKey, Runtime, SyncWrapper};
            let sk = Runtime::<AnyStorage, PoAConsensus, SyncWrapper>::generate_keypair()?;
            let pk = PublicKey::from(sk.verifying_key());
            Ok(text_or_json(
                json,
                &format!(
                    "Private key: {}\nPublic key: {}",
                    hex::encode(sk.to_bytes()),
                    hex::encode(pk.to_bytes())
                ),
                serde_json::json!({"private_key": hex::encode(sk.to_bytes()), "public_key": hex::encode(pk.to_bytes())}),
            ))
        }
        KeyCommands::Inspect { key_hex } => {
            let bytes = hex::decode(&key_hex)?;
            let msg = match bytes.len() {
                32 => {
                    use ed25519_dalek::SigningKey;
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    let sk = SigningKey::from_bytes(&arr);
                    let pk = baals::PublicKey::from(sk.verifying_key());
                    format!(
                        "Private key (32 bytes)\nCorresponding public key: {}",
                        hex::encode(pk.to_bytes())
                    )
                }
                64 => {
                    let mut arr = [0u8; 64];
                    arr.copy_from_slice(&bytes);
                    let sig = ed25519_dalek::Signature::from_bytes(&arr);
                    format!("Signature (64 bytes)\n{:?}", sig)
                }
                _ => format!("Key bytes ({} bytes)", bytes.len()),
            };
            Ok(text_or_json(json, &msg, serde_json::json!({"len": bytes.len()})))
        }
        KeyCommands::Sign { key_hex, message } => {
            use ed25519_dalek::Signer;
            use ed25519_dalek::SigningKey;
            let bytes = hex::decode(&key_hex)?;
            if bytes.len() != 32 {
                return Err("Private key must be 32 bytes hex".into());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            let sk = SigningKey::from_bytes(&arr);
            let sig = sk.sign(message.as_bytes());
            Ok(text_or_json(
                json,
                &format!("Signature: {}", hex::encode(sig.to_bytes())),
                serde_json::json!({"signature": hex::encode(sig.to_bytes())}),
            ))
        }
        KeyCommands::Verify { key_hex, message, signature } => {
            let pk = parse_pubkey(&key_hex)?;
            let sig_bytes = hex::decode(&signature)?;
            if sig_bytes.len() != 64 {
                return Err("Signature must be 64 bytes hex".into());
            }
            let mut sig_arr = [0u8; 64];
            sig_arr.copy_from_slice(&sig_bytes);
            let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
            let valid = pk.verify(message.as_bytes(), &sig).is_ok();
            Ok(text_or_json(
                json,
                if valid { "Signature VALID" } else { "Signature INVALID" },
                serde_json::json!({"valid": valid}),
            ))
        }
    }
}

fn api_base(endpoint: &str) -> String {
    let with_scheme = if endpoint.contains("://") {
        endpoint.to_string()
    } else {
        format!("http://{}", endpoint)
    };
    with_scheme.trim_end_matches('/').to_string()
}

fn parse_json_or_text(body: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(body)
        .unwrap_or_else(|_| serde_json::json!({ "raw": body }))
}

fn http_get(
    url: &str,
    bearer_token: Option<&str>,
) -> Result<(u16, String), Box<dyn std::error::Error>> {
    let mut req = ureq::get(url);
    if let Some(token) = bearer_token {
        req = req.set("Authorization", &format!("Bearer {}", token));
    }
    match req.call() {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().unwrap_or_default();
            Ok((status, body))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {}: {}", code, body).into())
        }
        Err(ureq::Error::Transport(e)) => Err(format!("HTTP transport error: {}", e).into()),
    }
}

fn http_post_json(
    url: &str,
    json_body: &str,
    bearer_token: Option<&str>,
) -> Result<(u16, String), Box<dyn std::error::Error>> {
    let mut req = ureq::post(url).set("Content-Type", "application/json");
    if let Some(token) = bearer_token {
        req = req.set("Authorization", &format!("Bearer {}", token));
    }
    match req.send_string(json_body) {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().unwrap_or_default();
            Ok((status, body))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {}: {}", code, body).into())
        }
        Err(ureq::Error::Transport(e)) => Err(format!("HTTP transport error: {}", e).into()),
    }
}

fn handle_api(action: cli::ApiCommands, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    match action {
        cli::ApiCommands::Health { endpoint } => {
            let url = format!("{}/health", api_base(&endpoint));
            let (status, body) = http_get(&url, None)?;
            Ok(text_or_json(
                json,
                &format!("GET {} -> HTTP {}", url, status),
                serde_json::json!({
                    "endpoint": url,
                    "status": status,
                    "response": parse_json_or_text(&body),
                }),
            ))
        }
        cli::ApiCommands::Submit { file, endpoint, token } => {
            let api_url = format!("{}/api/v1/transactions", api_base(&endpoint));
            let tx_json = std::fs::read_to_string(&file)?;
            let (status, body) = http_post_json(&api_url, &tx_json, Some(&token))?;
            Ok(text_or_json(
                json,
                &format!("POST {} -> HTTP {}", api_url, status),
                serde_json::json!({
                    "endpoint": api_url,
                    "status": status,
                    "response": parse_json_or_text(&body),
                }),
            ))
        }
        cli::ApiCommands::Deploy { wasm, deployer, gas_limit, init_hex, endpoint, token } => {
            let api_url = format!("{}/api/v1/contracts/deploy", api_base(&endpoint));
            let wasm_bytes = std::fs::read(&wasm)?;
            let payload = serde_json::json!({
                "deployer_hex": deployer,
                "wasm_hex": hex::encode(wasm_bytes),
                "init_hex": init_hex.unwrap_or_default(),
                "gas_limit": gas_limit,
            });
            let (status, body) = http_post_json(&api_url, &payload.to_string(), Some(&token))?;
            Ok(text_or_json(
                json,
                &format!("POST {} -> HTTP {}", api_url, status),
                serde_json::json!({
                    "endpoint": api_url,
                    "status": status,
                    "response": parse_json_or_text(&body),
                }),
            ))
        }
        cli::ApiCommands::Call { contract_id, method, args, endpoint, token } => {
            let api_url = format!("{}/api/v1/contracts/call", api_base(&endpoint));
            let payload_hex = hex::encode(args.unwrap_or_default().into_bytes());
            let payload = serde_json::json!({
                "contract_id": contract_id,
                "method": method,
                "payload_hex": payload_hex,
            });
            let (status, body) = http_post_json(&api_url, &payload.to_string(), token.as_deref())?;
            Ok(text_or_json(
                json,
                &format!("POST {} -> HTTP {}", api_url, status),
                serde_json::json!({
                    "contract_id": contract_id,
                    "endpoint": api_url,
                    "status": status,
                    "response": parse_json_or_text(&body),
                }),
            ))
        }
    }
}

fn handle_doctor(json: bool) -> Result<String, Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    let mut issues: Vec<String> = Vec::new();
    let mut info: Vec<String> = Vec::new();

    if let Some(home) = dirs::home_dir() {
        info.push(format!("Home dir: {:?}", home));
    } else {
        issues.push("Home directory not found".to_string());
    }

    let data_dir = PathBuf::from("./data");
    if data_dir.exists() {
        info.push(format!("Data dir: {:?} (exists)", data_dir));
    } else {
        info.push("Data dir: ./data (not yet initialized)".to_string());
    }

    let config_path = PathBuf::from("config.toml");
    if config_path.exists() {
        info.push("Config: config.toml (exists)".to_string());
    } else {
        info.push("Config: config.toml (not found, using defaults)".to_string());
    }

    let keystore_path = dirs::home_dir().map(|h| h.join(".baals/keys"));
    if let Some(ks_path) = &keystore_path {
        if ks_path.exists() {
            let count = std::fs::read_dir(ks_path).map(|e| e.count()).unwrap_or(0);
            info.push(format!("Keystore: {:?} ({} keys)", ks_path, count));
        } else {
            info.push("Keystore: not yet initialized".to_string());
        }
    }

    info.push("Storage backends: sled, redb".to_string());

    let healthy = issues.is_empty();
    let output = format!(
        "{}\n{}",
        info.join("\n"),
        if issues.is_empty() { String::new() } else { format!("\nIssues:\n{}", issues.join("\n")) }
    );

    Ok(text_or_json(
        json,
        &output,
        serde_json::json!({
            "healthy": healthy,
            "info": info,
            "issues": issues,
        }),
    ))
}
