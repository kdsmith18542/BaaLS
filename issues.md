# BaaLS Production Readiness Issues

Last audited: 2026-05-07

This file tracks confirmed production blockers and material readiness gaps found in the current repository state.

Severity legend:
- `P0`: Must fix before production use.
- `P1`: High-risk blocker; should be fixed before production rollout.
- `P2`: Important reliability/security/operability gap.

## Confirmed Findings (All Remediated)

### 1) [REMEDIATED] FFI null-pointer UB in contract call path
- **File**: `src/ffi.rs`
- **Fix**: `baals_sdk_call_contract` now includes a defensive null check for `args_ptr` and handles `args_len == 0` safely before creating slices.

### 2) [REMEDIATED] TLS config not wired into runtime startup
- **File**: `src/main.rs`
- **Fix**: TLS configuration is now loaded and applied to `CustomSync` during runtime construction.

### 3) [REMEDIATED] Peer sync/broadcast uses plaintext TCP
- **File**: `src/sync.rs`
- **Fix**: All outbound connections now use `connect_to_peer` which automatically upgrades to TLS if configured.

### 4) [REMEDIATED] mDNS/peer bootstrap disabled without explicit `--peer`
- **File**: `src/main.rs`
- **Fix**: The runtime now always uses `SyncWrapper` with a `CustomSync` instance if either peers or mDNS are enabled, ensuring mDNS-discovered peers can be added dynamically.

### 5) [REMEDIATED] Node status/backup/restore ignore selected backend
- **File**: `src/main.rs`
- **Fix**: Operational commands (`status`, `backup`, `restore`) now correctly respect the configured `StorageBackend`.

### 6) [REMEDIATED] Contract proof endpoint builds non-canonical proofs
- **File**: `src/main.rs`
- **Fix**: The endpoint now loads all keys for the contract from storage to build a canonical Sparse Merkle Tree, ensuring the generated proof is valid against the full state.

### 7) [REMEDIATED] `plan.md` quality-gate claims are stale
- **File**: `plan.md`
- **Fix**: Codebase has been brought into compliance with `cargo fmt` and `cargo clippy` (denying warnings), and benchmarks have been fixed to support CI/CD quality gates.

### 8) [REMEDIATED] Daemon startup drops network arguments
- **File**: `src/main.rs`
- **Fix**: Daemon relaunch logic now correctly forwards `--peer`, `--listen`, and `--mdns` flags to the background process.

### 9) [REMEDIATED] mDNS advertised port can mismatch actual listener port
- **File**: `src/main.rs`
- **Fix**: mDNS discovery now accurately announces the actual bound listener port derived from the `--listen` address.

### 10) [REMEDIATED] Config parsing failures are fail-open
- **File**: `src/main.rs`
- **Fix**: `Config::load` failures now result in an explicit error exit instead of falling back to silent defaults.

### 11) [REMEDIATED] Rate limiting is keyed by IP:port, not stable client identity
- **File**: `src/main.rs`
- **Fix**: Rate limiting now keys by the remote IP address only, preventing bypass via ephemeral source port cycling.

### 12) [REMEDIATED] FFI lock poisoning can panic outside catch boundary
- **File**: `src/ffi.rs`
- **Fix**: SDK Mutex locking is now performed inside the `catch_unwind` boundary and handles poisoning via `Result` instead of `unwrap()`, ensuring FFI boundary safety.

### 13) [REMEDIATED] Benchmark target is currently broken
- **File**: `benches/performance_benchmarks.rs`
- **Fix**: Updated the benchmark suite to align with the latest `ContractEngine` API, restoring full `cargo bench` functionality.

## Verification Evidence (2026-05-07)

- `cargo build --release` -> **PASS**
- `cargo test` -> **PASS**
- `cargo fmt -- --check` -> **PASS**
- `cargo clippy --all-targets -- -D warnings` -> **PASS**
- `cargo bench --no-run` -> **PASS**
