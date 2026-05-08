# Security Policy

## Supported Versions

BaaLS is currently pre-production. No stable releases exist yet.

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

**DO NOT OPEN A PUBLIC ISSUE** for security vulnerabilities.

Report security issues to the project maintainers via the
[GitHub Security Advisory](https://github.com/YOUR_ORG/baals/security/advisories/new) page.

You can expect:
- Acknowledgment within 72 hours
- Status update within 5 business days
- Disclosure coordination with you

## Threat Model

BaaLS executes untrusted WebAssembly (WASM) smart contracts in a sandboxed
runtime. The primary security boundaries are:

1. **WASM Sandbox** — Contracts must not escape the wasmtime runtime,
   must not exhaust host resources, and must not access other contracts'
   state without explicit authorization.

2. **Consensus / P2P** — Blocks and transactions must be cryptographically
   verified. P2P connections must be authenticated (TLS + cert pinning
   or Ed25519 challenge-response).

3. **Key Management** — Private keys must be encrypted at rest using
   AES-256-GCM with strong KDF (PBKDF2 >= 600k iterations). Key files
   must be created with 0600 permissions and atomic writes.

4. **Storage Integrity** — The Merkle state root must always match the
   canonical state. All state transitions must be atomic per block.

## Security Hardening Checklist

- [x] WASM gas metering and resource limits
- [x] Float opcode ban for determinism
- [x] Reentrancy guard (contract call depth tracking)
- [x] Call depth limit (max 64)
- [x] Atomic block application (contract state in block batch)
- [x] Host function gas exhaustion aborts execution
- [x] Consensus validation on all import paths (sync, fork, direct)
- [x] P2P Ed25519 challenge-response handshake
- [x] P2P TLS with certificate pinning
- [x] Keystore PBKDF2 hardening (600k iterations)
- [x] Keystore atomic writes + 0600 permissions (Unix)
- [x] Keystore symlink rejection
- [x] Keystore plaintext zeroization
- [x] FFI shutdown and reinit detection
- [x] CI: cargo fmt, clippy -D warnings, cargo test, cargo audit
- [ ] Fuzzing for transaction decoding, block import, sync messages
- [ ] Argon2id/scrypt migration path for keystore KDF
- [ ] Formal WASM security audit

## Dependencies

Dependencies are audited via `cargo audit` in CI. Critical dependencies:

- `wasmtime` — WASM runtime sandbox (43.x line)
- `ed25519-dalek` — Ed25519 signatures
- `aes-gcm` / `pbkdf2` — Keystore encryption
- `rustls` / `tokio-rustls` — TLS for P2P
- `sled` / `redb` — Storage backends

## Responsible Disclosure

We follow a coordinated disclosure process:

1. Reporter submits vulnerability privately
2. Maintainers confirm and develop a fix
3. A patch release is prepared
4. CVE is requested (if applicable)
5. Public disclosure coordinated with reporter
