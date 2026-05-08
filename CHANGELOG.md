# Changelog

## [0.1.0] — Unreleased (pre-production)

### Security
- **P0**: WASM host function gas exhaustion now properly aborts execution instead of silently continuing.
- **P0**: Contract deployment state changes (code, deployer, init side effects) now committed atomically in block batch.
- **P0**: P2P TlsConfig::load now pins CA certificates from `ca_cert_path` parameter.
- **P0**: Consensus validation enforced on all block import paths (sync, fork, direct).
- **P1**: Keystore PBKDF2 iterations increased to 600,000; SALT_LEN increased to 32 bytes.
- **P1**: Keystore atomic writes (temp file + rename), Unix 0600 permissions, symlink rejection, full plaintext zeroization.
- **P1**: FFI: `baals_sdk_submit_tx` now uses proper string parsing; distinct reinit error code; added `baals_sdk_shutdown` and `baals_sdk_version`.
- **P1**: Go SDK input validation (regex) on binPath/dataDir to prevent command injection.
- **P1**: NodeJS native SDK path traversal prevention (rejects `..` in data_dir).
- **P1**: Kubernetes deployment `runAsNonRoot: true` security context.
- **P1**: GitHub Actions pinned to commit SHAs for supply-chain security.
- **P2**: Mempool byte accounting uses `payload_size_estimate` instead of `size_of_val`.
- **P2**: All mempool/consensus arithmetic uses `saturating_add`/`saturating_sub`.
- Added `--allow-insecure-dev-network` flag (must be explicitly set for non-TLS P2P).
- Added `deny.toml` for cargo-deny license/security/dependency scanning.
- Added `SECURITY.md` with threat model, disclosure process, hardening checklist.

### Added
- CI: `cargo audit`, `cargo deny`, `--all-features` tests, `--release` tests.
- Wasmtime upgraded from 27.0 to 43.0.1.
- sha2 upgraded to 0.11.0.
- criterion upgraded to 0.8.2.
- actions/checkout upgraded to v6.

### Fixed
- Pre-existing compilation errors: SparseMerkleTree API, LedgerError variants, TransactionPayload::Data match arm, contract_events_tree field, duplicate bincode import.
- Spec_Compliance_Notes.md header corrected (12 resolved, 10 WONTFIX deviations).
- Cargo.lock cranelift-codegen at 0.130.2 (AIKIDO-2025-10778 false positive — already patched).

### Dependencies
| Crate | Version | Notes |
|-------|---------|-------|
| wasmtime | 43.0.1 | WASM runtime sandbox |
| ed25519-dalek | 2.2 | Ed25519 signatures |
| sled | 0.34 | Default storage backend |
| redb | 2.x | Alternative storage backend |
| tokio | 1.x | Async runtime |
| rustls | 0.23 | TLS for P2P |
| aes-gcm / pbkdf2 | 0.10 / 0.12 | Keystore encryption |
| sha2 | 0.11 | Hashing |
