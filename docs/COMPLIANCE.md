# BaaLS Spec Compliance

Last verified: 2026-05-19

## Verification Snapshot

The following commands were run on this repository state:

- `cargo fmt --all -- --check` -> PASS
- `cargo clippy --all-targets --all-features -- -D warnings` -> PASS
- `cargo test --all-features` -> PASS

## Test Summary (`cargo test --all-features`)

- Total passed: 77
- Total failed: 0
- Total ignored: 1 (`tests/cli_daemon.rs` marked flaky on Windows harness)

Breakdown:

- Unit tests (`src/lib.rs`): 18 passed
- CLI lifecycle integration: 1 passed
- CQ regression suite: 10 passed
- Golden tests: 1 passed
- Integration tests: 44 passed
- Security hardening tests: 3 passed

## HTTP API Status

Canonical namespace is `/api/v1/*` with legacy aliases retained for compatibility.

Implemented canonical endpoints:

- `POST /api/v1/transactions`
- `GET /api/v1/blocks/{height}`
- `GET /api/v1/blocks/latest`
- `GET /api/v1/blocks/hash/{hash}`
- `GET /api/v1/accounts/{addr}`
- `POST /api/v1/contracts/call` (read-only)
- `GET /api/v1/health` (alias of `/health`)

Mutating endpoints require `Authorization: Bearer <jwt>` issued by `POST /auth/token` and are loopback-restricted.

## Notable Deviations / Scope Limits

- L1: Crate name is `baals` (spec placeholder `libchain`)
- L2: Hash fields use `[u8; 32]` instead of `String`
- L3: Metadata uses `BTreeMap<String, String>` instead of `Map<String, Value>`
- L4: `recipient` uses `Address` enum (wallet or contract)
- L7: `Runtime::start()` does not auto-produce blocks unless configured
- L8: Multi-signer authorized key set exists, but quorum-threshold multi-validator PoA is not fully implemented
- L9: P2P sync model is polling-oriented (not push-streaming)
- L10: No WASI Preview 2 socket support in contract model

## CLI Coverage (Current)

- Fully implemented core groups: `node`, `wallet`, `tx`, `query`, `db`, `dev`, `key`, `proof`, `doctor`
- Partially implemented / planned subcommands: selected `p2p`, `contract`, `admin`, `api` advanced flows are explicitly marked not implemented where runtime wiring is missing

## Reproducibility

To regenerate this status:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
