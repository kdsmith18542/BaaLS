use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
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
    let pid_path = data_dir.join("baals.pid");
    let stop_path = data_dir.join("baals.stop");

    let child = Command::new(bin_path)
        .arg("node")
        .arg("start")
        .arg("--data-dir")
        .arg(data_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn node start");
    let mut child = ChildGuard { child };

    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(200), || { pid_path.exists() }),
        "pid file should exist after start"
    );

    let status_running = Command::new(bin_path)
        .arg("node")
        .arg("status")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .output()
        .expect("status while running");
    assert!(status_running.status.success(), "status command should succeed");
    let running_json: Value =
        serde_json::from_slice(&status_running.stdout).expect("parse running status json");
    assert_eq!(running_json["running"].as_bool(), Some(true));

    // API contract smoke checks: canonical /api/v1 aliases should be reachable.
    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
            http_request("GET", "127.0.0.1:8080", "/health", None, &[])
                .map(|(status, _)| status == 200)
                .unwrap_or(false)
        }),
        "health endpoint should be reachable"
    );

    let (health_v1_status, _) = http_request("GET", "127.0.0.1:8080", "/api/v1/health", None, &[])
        .expect("GET /api/v1/health");
    assert_eq!(health_v1_status, 200, "/api/v1/health should return 200");

    let (latest_status, latest_body) =
        http_request("GET", "127.0.0.1:8080", "/api/v1/blocks/latest", None, &[])
            .expect("GET /api/v1/blocks/latest");
    assert_eq!(latest_status, 200, "/api/v1/blocks/latest should return 200");
    let latest_json: Value = serde_json::from_str(&latest_body).expect("parse latest block json");
    assert!(
        latest_json.get("height").is_some(),
        "latest block response should include height"
    );

    let (by_height_status, by_height_body) =
        http_request("GET", "127.0.0.1:8080", "/api/v1/blocks/0", None, &[])
            .expect("GET /api/v1/blocks/0");
    assert_eq!(by_height_status, 200, "/api/v1/blocks/<height> should return 200");
    let by_height_json: Value =
        serde_json::from_str(&by_height_body).expect("parse block-by-height json");
    assert_eq!(by_height_json["height"].as_u64(), Some(0));

    let (account_status, _) =
        http_request("GET", "127.0.0.1:8080", "/api/v1/accounts/not-a-pubkey", None, &[])
            .expect("GET /api/v1/accounts/<addr>");
    assert_eq!(
        account_status, 404,
        "invalid account key should return non-success from account query endpoint"
    );

    let (contract_call_status, _) =
        http_request("POST", "127.0.0.1:8080", "/api/v1/contracts/call", Some("{}"), &[])
            .expect("POST /api/v1/contracts/call");
    assert_eq!(
        contract_call_status, 400,
        "contract call endpoint should be routed and validate payload"
    );

    let (tx_submit_status, _) =
        http_request("POST", "127.0.0.1:8080", "/api/v1/transactions", Some("{}"), &[])
            .expect("POST /api/v1/transactions");
    assert_eq!(
        tx_submit_status, 503,
        "mutating endpoint should require BAALS_ADMIN_TOKEN when not configured"
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
