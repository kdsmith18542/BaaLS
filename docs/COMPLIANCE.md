# BaaLS Spec Compliance

> Auto-generated summary from CI artifacts. Last updated: 2026-05-09

## CI Status

| Check | Status |
|-------|--------|
| `cargo fmt --check` | ✅ |
| `cargo clippy -D warnings` | ✅ |
| `cargo test --lib` | ✅ 18/18 pass |
| `cargo test --all-features` | ✅ 63 pass (18 unit + 44 integration + 1 golden) |
| `cargo test --release` | ✅ |
| `cargo audit` | ✅ |
| `cargo deny check` | ✅ |
| Fuzz targets | 7 targets (4 new) |
| Golden tests | ✅ |

## Resolved Gaps

| Gap | Status | Notes |
|-----|--------|-------|
| Block hash uses tx Merkle root | ✅ RESOLVED | `Block::calculate_hash()` computes MerkleTree from tx hashes |
| ContractDeploy missing init_payload | ✅ RESOLVED | Added `init_payload: Option<Vec<u8>>` |
| WASI block context always zero | ✅ RESOLVED | Added `block_index`/`block_timestamp` params |
| baals_call_contract missing value param | ✅ RESOLVED | Added `value: i64` to WASI host function |
| Config not wired to consensus | ✅ RESOLVED | `build_runtime()` passes `block_time_ms` from config |
| Block overflow tx allowed | ✅ RESOLVED | Tx exceeding block gas/size limits properly rejected |
| Auto-block threshold bypass | ✅ RESOLVED | Blocks only auto-produce at configured threshold |
| Missing wallet delete CLI | ✅ RESOLVED | Added `WalletCommands::Delete` |
| Config::set() missing keys | ✅ RESOLVED | Added all storage/network config keys |

## Production Hardening (P0/P1)

| Item | Status | Notes |
|------|--------|-------|
| Gas pricing | ✅ | `gas_price` field, fee deducted as `gas_price * gas_used` |
| Time-windowed rate limiting | ✅ | 10 tx/s per sender + 100 pending cap |
| Keystore KDF | ✅ | Argon2id v2 (new), PBKDF2 v1 (legacy compat) |
| Fuzz targets | ✅ | 7 targets covering tx, block, WASM, ledger, sync, merkle |
| justfile / release infra | ✅ | `just ci`, `just release`, `scripts/test-all.*` |

## WONTFIX Deviations

| ID | Deviation | Rationale |
|----|-----------|-----------|
| L1 | Crate named `baals` not `libchain` | Simpler, more recognizable |
| L2 | Hash fields use `[u8; 32]` not `String` | Type-safe, deterministic layout |
| L3 | Metadata uses `BTreeMap<String, String>` | Flat, ordered, no serde_json::Value overhead |
| L4 | `recipient` is `Address` enum not `PublicKey` | Targets both wallets and contracts |
| L7 | `start()` doesn't auto-produce blocks | Tests/embedded SDK need control |
| L8 | Single-validator PoA only | Multi-validator deferred |
| L9 | P2P sync: polling (not push) | Simpler, sufficient for single-validator |
| L10 | WASM: no WASI preview 2 sockets | Not needed for contract model |

## CLI Coverage

| Group | Commands | Status |
|-------|----------|--------|
| `node` | start, stop, status, config, backup, restore | ✅ Complete |
| `wallet` | create, list, show, import, export, export-public, sign, verify, delete, rotate, change-password, recover | ✅ Complete |
| `tx` | transfer, deploy-contract, call-contract, data, sign, submit, inspect, validate, estimate-fee, decode | ✅ Complete |
| `query` | head, block, tx, account, contract-state, contract-call, blocks, txs-by-account, txs-by-contract, mempool | ✅ Complete |
| `db` | version, migrate, verify, compact, backup, restore, check-indexes, rebuild-indexes, export, import | ✅ Complete |
| `dev` | generate-keys, simulate-contract, validate-tx, storage-stats, performance-report, validate-chain, monitor, dump-state, replay-block, replay-chain, fuzz-wasm, inspect-wasm, verify-merkle-root, repair-indexes | ✅ Complete |
| `key` | generate, inspect, sign, verify | ✅ Complete |
| `proof` | account, contract, verify | ✅ Complete |
| `doctor` | diagnostics | ✅ Complete |

## Test Coverage

| Area | Status |
|------|--------|
| Unit tests (types, ledger, consensus) | ✅ 18 pass |
| Integration tests (state transitions) | ✅ 44 pass |
| WASM security tests | ✅ 3 pass |
| Golden tests | ✅ 1 pass |
| Fuzz targets | ✅ 7 configured |
| CLI end-to-end | ✅ 1 pass |
| P2P multi-node | ✅ Verified in integration suite |
| Crash recovery | ✅ Verified in integration suite |
| Load/soak | ✅ Performance benchmarks pass |
