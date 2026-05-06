use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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

fn run_command_json(
    bin_path: &str,
    data_dir: &std::path::Path,
    args: &[&str],
    timeout: Duration,
) -> Value {
    let mut cmd = Command::new(bin_path);
    cmd.args(args)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().expect("spawn command");
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait command") {
            let mut stdout = Vec::new();
            if let Some(mut pipe) = child.stdout.take() {
                let _ = pipe.read_to_end(&mut stdout);
            }
            assert!(status.success(), "command should succeed: {:?}", args);
            return serde_json::from_slice(&stdout).expect("parse command json");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out: {:?}", args);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn node_status_json(bin_path: &str, data_dir: &std::path::Path) -> Value {
    run_command_json(
        bin_path,
        data_dir,
        &["node", "status"],
        Duration::from_secs(10),
    )
}

#[test]
#[ignore = "Flaky under Windows test harness with detached daemon process"]
fn test_cli_node_daemon_start_and_stop() {
    let bin_path = env!("CARGO_BIN_EXE_baals");
    let data_dir: PathBuf = TempDir::new().expect("create temp dir").keep();

    let start_json = run_command_json(
        bin_path,
        &data_dir,
        &["node", "start", "--daemon"],
        Duration::from_secs(15),
    );
    assert_eq!(
        start_json["status"].as_str(),
        Some("daemon_start_requested")
    );

    let started = wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
        node_status_json(bin_path, &data_dir)["running"].as_bool() == Some(true)
    });
    assert!(started, "daemon should report running state");

    let stop_json = run_command_json(
        bin_path,
        &data_dir,
        &["node", "stop"],
        Duration::from_secs(10),
    );
    let stop_status = stop_json["status"].as_str().unwrap_or_default();
    assert!(
        matches!(stop_status, "stop_requested" | "already_stopped"),
        "unexpected stop status: {}",
        stop_status
    );

    let stopped = wait_until(Duration::from_secs(15), Duration::from_millis(250), || {
        node_status_json(bin_path, &data_dir)["running"].as_bool() == Some(false)
    });
    assert!(stopped, "daemon should stop after stop signal");
}
