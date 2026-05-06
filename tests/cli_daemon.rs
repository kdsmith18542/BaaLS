use serde_json::Value;
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

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

fn node_status_json(bin_path: &str, data_dir: &std::path::Path) -> Value {
    let output = Command::new(bin_path)
        .arg("node")
        .arg("status")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .output()
        .expect("node status");
    assert!(output.status.success(), "node status should succeed");
    serde_json::from_slice(&output.stdout).expect("parse node status json")
}

#[test]
fn test_cli_node_daemon_start_and_stop() {
    let bin_path = env!("CARGO_BIN_EXE_baals");
    let temp_dir = TempDir::new().expect("create temp dir");
    let data_dir = temp_dir.path();

    let start_output = Command::new(bin_path)
        .arg("node")
        .arg("start")
        .arg("--daemon")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .output()
        .expect("node start --daemon");
    assert!(
        start_output.status.success(),
        "daemon start command should succeed"
    );
    let start_json: Value =
        serde_json::from_slice(&start_output.stdout).expect("parse daemon start json");
    assert_eq!(
        start_json["status"].as_str(),
        Some("daemon_start_requested")
    );

    let started = wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
        node_status_json(bin_path, data_dir)["running"].as_bool() == Some(true)
    });
    assert!(started, "daemon should report running state");

    let stop_output = Command::new(bin_path)
        .arg("node")
        .arg("stop")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .output()
        .expect("node stop");
    assert!(stop_output.status.success(), "node stop should succeed");
    let stop_json: Value = serde_json::from_slice(&stop_output.stdout).expect("parse stop json");
    let stop_status = stop_json["status"].as_str().unwrap_or_default();
    assert!(
        matches!(stop_status, "stop_requested" | "already_stopped"),
        "unexpected stop status: {}",
        stop_status
    );

    let stopped = wait_until(Duration::from_secs(15), Duration::from_millis(250), || {
        node_status_json(bin_path, data_dir)["running"].as_bool() == Some(false)
    });
    assert!(stopped, "daemon should stop after stop signal");
}
