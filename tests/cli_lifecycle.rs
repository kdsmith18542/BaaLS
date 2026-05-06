use serde_json::Value;
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

#[test]
fn test_cli_node_lifecycle_start_status_stop() {
    let bin_path = env!("CARGO_BIN_EXE_baals");
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
        wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
            pid_path.exists()
        }),
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
    assert!(
        status_running.status.success(),
        "status command should succeed"
    );
    let running_json: Value =
        serde_json::from_slice(&status_running.stdout).expect("parse running status json");
    assert_eq!(running_json["running"].as_bool(), Some(true));

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
    assert!(
        status_stopped.status.success(),
        "status command should succeed"
    );
    let stopped_json: Value =
        serde_json::from_slice(&status_stopped.stdout).expect("parse stopped status json");
    assert_eq!(stopped_json["running"].as_bool(), Some(false));

    assert!(
        !Path::new(&pid_path).exists(),
        "pid file should be removed after shutdown"
    );
    assert!(
        !Path::new(&stop_path).exists(),
        "stop signal file should be removed after shutdown"
    );
}
