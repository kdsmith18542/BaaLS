use ed25519_dalek::Signer;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn wait_until(timeout: Duration, interval: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(interval);
    }
    check()
}

fn pick_free_local_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral test port");
    listener.local_addr().expect("read ephemeral test port").port()
}

fn http_request(
    method: &str,
    host: &str,
    path: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> Result<(u16, String), String> {
    let mut stream = TcpStream::connect(host).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;

    let body_text = body.unwrap_or("");
    let mut request =
        format!("{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n", method, path, host);
    for (k, v) in headers {
        request.push_str(&format!("{}: {}\r\n", k, v));
    }
    if !body_text.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n{}", body_text.len(), body_text));

    stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;

    let mut lines = response.lines();
    let status_line = lines.next().ok_or("empty response")?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or("missing status code")?
        .parse::<u16>()
        .map_err(|e| e.to_string())?;

    let body_part = response.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Ok((status_code, body_part))
}

#[test]
fn test_cli_node_lifecycle_start_status_stop() {
    let bin_path = env!("CARGO_BIN_EXE_baalsd");
    let temp_dir = TempDir::new().expect("create temp dir");
    let data_dir = temp_dir.path();
    let api_port = pick_free_local_port();
    let api_host = format!("127.0.0.1:{}", api_port);
    let mut p2p_port = pick_free_local_port();
    while p2p_port == api_port {
        p2p_port = pick_free_local_port();
    }
    let p2p_listen = format!("127.0.0.1:{}", p2p_port);
    let pid_path = data_dir.join("baals.pid");
    let stop_path = data_dir.join("baals.stop");
    let node_sk_hex = "1111111111111111111111111111111111111111111111111111111111111111";
    let node_sk_bytes = hex::decode(node_sk_hex).expect("decode test node private key");
    let mut node_sk_arr = [0u8; 32];
    node_sk_arr.copy_from_slice(&node_sk_bytes);
    let node_sk = ed25519_dalek::SigningKey::from_bytes(&node_sk_arr);
    let node_pk_hex = hex::encode(node_sk.verifying_key().to_bytes());

    let child = Command::new(bin_path)
        .arg("node")
        .arg("start")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--port")
        .arg(api_port.to_string())
        .arg("--listen")
        .arg(p2p_listen)
        .env("BAALS_CONSENSUS_KEY", node_sk_hex)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn node start");
    let mut child = ChildGuard { child };

    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(200), || { pid_path.exists() }),
        "pid file should exist after start"
    );

    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
            let status_running = Command::new(bin_path)
                .arg("node")
                .arg("status")
                .arg("--data-dir")
                .arg(data_dir)
                .arg("--json")
                .output();
            match status_running {
                Ok(output) if output.status.success() => {
                    serde_json::from_slice::<Value>(&output.stdout)
                        .ok()
                        .and_then(|v| v["running"].as_bool())
                        == Some(true)
                }
                _ => false,
            }
        }),
        "node should report running=true shortly after start"
    );

    // API contract smoke checks: canonical /api/v1 aliases should be reachable.
    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
            http_request("GET", api_host.as_str(), "/health", None, &[])
                .map(|(status, _)| status == 200)
                .unwrap_or(false)
        }),
        "health endpoint should be reachable"
    );

    let (health_v1_status, _) = http_request("GET", api_host.as_str(), "/api/v1/health", None, &[])
        .expect("GET /api/v1/health");
    assert_eq!(health_v1_status, 200, "/api/v1/health should return 200");

    let (latest_status, latest_body) =
        http_request("GET", api_host.as_str(), "/api/v1/blocks/latest", None, &[])
            .expect("GET /api/v1/blocks/latest");
    assert_eq!(latest_status, 200, "/api/v1/blocks/latest should return 200");
    let latest_json: Value = serde_json::from_str(&latest_body).expect("parse latest block json");
    assert!(
        latest_json.get("height").is_some(),
        "latest block response should include height"
    );

    let (by_height_status, by_height_body) =
        http_request("GET", api_host.as_str(), "/api/v1/blocks/0", None, &[])
            .expect("GET /api/v1/blocks/0");
    assert_eq!(by_height_status, 200, "/api/v1/blocks/<height> should return 200");
    let by_height_json: Value =
        serde_json::from_str(&by_height_body).expect("parse block-by-height json");
    assert_eq!(by_height_json["height"].as_u64(), Some(0));

    let (account_status, _) =
        http_request("GET", api_host.as_str(), "/api/v1/accounts/not-a-pubkey", None, &[])
            .expect("GET /api/v1/accounts/<addr>");
    assert_eq!(
        account_status, 404,
        "invalid account key should return non-success from account query endpoint"
    );

    let (contract_call_status, _) =
        http_request("POST", api_host.as_str(), "/api/v1/contracts/call", Some("{}"), &[])
            .expect("POST /api/v1/contracts/call");
    assert_eq!(
        contract_call_status, 400,
        "contract call endpoint should be routed and validate payload"
    );

    let (tx_submit_status, _) =
        http_request("POST", api_host.as_str(), "/api/v1/transactions", Some("{}"), &[])
            .expect("POST /api/v1/transactions");
    assert_eq!(
        tx_submit_status, 401,
        "mutating endpoint should require a JWT Authorization header"
    );

    // Avoid triggering per-IP burst limits in the test server.
    std::thread::sleep(Duration::from_millis(1100));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("unix timestamp")
        .as_secs();
    let nonce = "cli_lifecycle_nonce";
    let challenge = format!("baals-auth-token:{}:{}", now, nonce);
    let sig = node_sk.sign(challenge.as_bytes());
    let token_req = serde_json::json!({
        "timestamp": now,
        "nonce": nonce,
        "ttl_seconds": 300,
        "public_key": node_pk_hex,
        "signature": hex::encode(sig.to_bytes())
    });
    let (token_status, token_body) =
        http_request("POST", api_host.as_str(), "/auth/token", Some(&token_req.to_string()), &[])
            .expect("POST /auth/token");
    assert_eq!(token_status, 200, "/auth/token should return 200");
    let token_json: Value = serde_json::from_str(&token_body).expect("parse /auth/token json");
    let token = token_json["token"].as_str().expect("token in /auth/token response");
    let auth_header = format!("Bearer {}", token);

    // Avoid triggering per-IP burst limits in the test server.
    std::thread::sleep(Duration::from_millis(1100));
    let (authed_submit_status, _) = http_request(
        "POST",
        api_host.as_str(),
        "/api/v1/transactions",
        Some("{}"),
        &[("Authorization", auth_header.as_str())],
    )
    .expect("POST /api/v1/transactions with JWT");
    assert_eq!(
        authed_submit_status, 400,
        "authorized submit with invalid payload should pass auth and fail payload validation"
    );

    let missing_tx_hash = "0000000000000000000000000000000000000000000000000000000000000000";
    let (tx_lookup_status, _) = http_request(
        "GET",
        api_host.as_str(),
        &format!("/api/v1/transactions/{}", missing_tx_hash),
        None,
        &[],
    )
    .expect("GET /api/v1/transactions/{hash}");
    assert_eq!(tx_lookup_status, 404, "unknown tx lookup should return 404");

    let (tx_finality_status, _) = http_request(
        "GET",
        api_host.as_str(),
        &format!("/api/v1/transactions/{}/finality", missing_tx_hash),
        None,
        &[],
    )
    .expect("GET /api/v1/transactions/{hash}/finality");
    assert_eq!(tx_finality_status, 404, "unknown tx finality lookup should return 404");

    // Avoid triggering per-IP burst limits in the test server.
    std::thread::sleep(Duration::from_millis(1100));

    let (supply_status, supply_body) =
        http_request("GET", api_host.as_str(), "/api/v1/supply", None, &[])
            .expect("GET /api/v1/supply");
    assert_eq!(supply_status, 200, "/api/v1/supply should return 200");
    let supply_json: Value = serde_json::from_str(&supply_body).expect("parse supply json");
    assert!(
        supply_json.get("total_supply").is_some(),
        "supply response should include total_supply"
    );

    let (metrics_status, metrics_body) =
        http_request("GET", api_host.as_str(), "/metrics", None, &[]).expect("GET /metrics");
    assert_eq!(metrics_status, 200, "/metrics should return 200");
    assert!(
        metrics_body.contains("chain_height"),
        "/metrics payload should include chain_height gauge"
    );

    let stop_output = Command::new(bin_path)
        .arg("node")
        .arg("stop")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .output()
        .expect("stop command");
    assert!(stop_output.status.success(), "stop command should succeed");
    let stop_json: Value = serde_json::from_slice(&stop_output.stdout).expect("parse stop json");
    let stop_status = stop_json["status"].as_str().unwrap_or_default();
    assert!(
        matches!(stop_status, "stop_requested" | "already_stopped"),
        "unexpected stop status: {}",
        stop_status
    );

    assert!(
        wait_until(Duration::from_secs(15), Duration::from_millis(250), || {
            child.try_wait().ok().flatten().is_some()
        }),
        "node process should exit after stop signal"
    );

    let status_stopped = Command::new(bin_path)
        .arg("node")
        .arg("status")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .output()
        .expect("status after stop");
    assert!(status_stopped.status.success(), "status command should succeed");
    let stopped_json: Value =
        serde_json::from_slice(&status_stopped.stdout).expect("parse stopped status json");
    assert_eq!(stopped_json["running"].as_bool(), Some(false));

    assert!(!Path::new(&pid_path).exists(), "pid file should be removed after shutdown");
    assert!(
        !Path::new(&stop_path).exists(),
        "stop signal file should be removed after shutdown"
    );
}
