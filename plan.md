# BaaLS Development Plan

**ALL CODE MUST BE PRODUCTION GRADE. NO STUBS WITHOUT EXPLICIT APPROVAL.**

**Last Updated**: 2026-05-06
**Project Status**: Phases 1-9 complete (core engine functional, all CLI/SDK/FFI implemented). 33 tests pass (12 lib + 1 cli lifecycle + 20 integration), 1 ignored daemon harness test. **31 spec-to-code gaps identified** — see Phase 10 roadmap below.
**Compliance**: All implementation must satisfy the specifications in `docs/`.

---

## 1. Specification Compliance Map

Accurate status per full spec-to-code audit conducted 2026-05-06.

| Spec Document | Governs | Primary Source Files | Status | Critical Gaps |
|---|---|---|---|---|
| `docs/BaaLS_Overview.md` | Architecture, use cases, core principles | `src/lib.rs`, `README.md` | Near Complete | Crate named `baals` not `libchain` (minor) |
| `docs/BaaLS_Core_Engine_Runtime.md` | Runtime API, data flow, module boundaries | `src/runtime.rs`, `src/types.rs` | Near Complete | No `WasmRuntime` subtrait; field type differences (hash strings vs bytes) |
| `docs/BaaLS_Ledger_State_Transition.md` | Block validation, state transitions, Merkle roots | `src/ledger.rs` | Near Complete | Nonce gaps rejected |
| `docs/BaaLS_Storage_Layer.md` | Storage trait, key-value schema, indexing, atomicity | `src/storage.rs` | Near Complete | Key prefix mismatches; `compact()` doesn't actually compact; sled config not applied; WAL implemented but per-tree not cross-DB |
| `docs/BaaLS_Consensus_Engine_PoA.md` | PoA consensus, block signing, validation | `src/consensus.rs` | Partial | Block nonce unvalidated; no fork resolution execution; no multi-validator support |
| `docs/BaaLS_Transaction_Mempool.md` | Tx format, canonical serialization, verification, mempool | `src/types.rs`, `src/runtime.rs` | Near Complete | TTL 5min vs spec 24h; eviction missing least-gas tier; nonce gaps rejected vs queued |
| `docs/BaaLS_Smart_Contract_Module.md` | WASM runtime, WASI host functions, gas metering | `src/contracts.rs` | Near Complete | Float opcodes not banned; no export/import whitelist validation; gas estimation stub; no capability-based security; events not persisted |
| `docs/BaaLS_CLI_SDK_Wiring_Overview.md` | CLI commands, SDK APIs, FFI bindings | `src/main.rs`, `src/sdk.rs`, `src/ffi.rs` | Near Complete | No cdylib target; NodeJS SDK types-only (no native addon); Go SDK needs compiled `.so`/`.dll` to link |
| `docs/BaaLS_Production_Readiness_Guide.md` | Deployment, monitoring, security, operations | `src/metrics.rs`, `docs/` | Partial | No TLS; no certificate pinning; no rate limiting; no log rotation; `compact()` is a no-op |

---

## 2. Current State Assessment

### 2.1 What Actually Works (Verified)

- **Types & Cryptography** (`src/types.rs`): `Block`, `Transaction`, `ChainState`, `Account`, `Address`, `ContractId` + Ed25519 + SHA256 + Blake3 + MerkleTree (dense, `RefCell` cached root with `&self` access)
- **Storage** (`src/storage.rs`): `Storage` trait with 25+ methods, `SledStorage` multi-tree schema, WAL-based atomic batch recovery, backup/restore to separate sled DB, advanced indexing (address, contract, block, height)
- **Ledger** (`src/ledger.rs`): Genesis initialization, block validation (index, prev_hash, hash, timestamp sequence, tx signatures), state transition with per-tx gas accounting, `StateTransitionError` distinct type, contract deploy/call routed through engine
- **Consensus** (`src/consensus.rs`): `ConsensusEngine` trait, `PoAConsensus` with mandatory block signing (metadata + ed25519), `BlockGasLimit` (30M) + `BlockSizeLimit` (10MB), persistent consensus key
- **Runtime** (`src/runtime.rs`): `Runtime<S, C, Y>` with mempool (HashMap + BTreeMap, TTL eviction, priority ordering, per-sender rate limiting), `submit_transaction()` with full validation, `produce_block()` with broadcast
- **Contracts** (`src/contracts.rs`): 15 WASM host functions (storage, crypto, events, inter-contract calls, memory growth), wasmtime fuel metering, reentrancy guard (`HashMap<ContractId, u32>` + `AtomicU32`), bounds-checked WASM inputs (1MB max), bytecode validation (magic bytes + size + pre-compilation)
- **Keystore** (`src/keystore.rs`): PBKDF2-HMAC-SHA256 (100k iterations) + AES-256-GCM, create/list/import/export/sign operations
- **Sync** (`src/sync.rs`): `SyncLayer` trait, `CustomSync` with TCP P2P, 16 message variants, length-prefixed bincode framing, fork detection, `NoopSync` default
- **Metrics** (`src/metrics.rs`): `MetricsCollector` with TPS, latency percentiles, `HealthStatus`, `OptimizationReport`, background monitoring
- **Config** (`src/config.rs`): TOML with 5 sections (node, consensus, storage, network, logging), load chain (env → explicit → files → default), `save`/`validate`/`set`, JSON logging support
- **CLI** (`src/main.rs`): 25+ subcommands across `node`/`wallet`/`tx`/`query`/`dev`, `--json` global flag, persistent `data-dir` throughout
- **SDK/FFI** (`src/sdk.rs`, `src/ffi.rs`, `sdk/go/`, `sdk/nodejs/`): Rust SDK with builder pattern, C ABI via `OnceLock<Mutex<BaaLSSdk>>`, Go CGo declarations (corrected signatures), Node.js TypeScript definitions
- **DevOps** (`Dockerfile`, `k8s/deployment.yaml`): Multi-stage Docker build, K8s ConfigMap/Deployment/Service with probes

### 2.2 Resolved Bugs (Historical)

All 18 bugs from the Phase 9 audit have been fixed:
1. WASM host function unbounded allocation (bounds-checked at 1MB)
2. Inter-contract call results discarded (populated in `HostState.inter_contract_results`)
3. Sync protocol deserialization broken (direct `bincode::deserialize`)
4. FFI module not in `lib.rs` (added `pub mod ffi`)
5. SDK invalid `PublicKey::from_bytes(&[1u8; 32])` (random keypair generation)
6. Network message max size unchecked (16MB limit)
7. Nonce gap vulnerability (accepts `>= expected` instead of strict `==`)
8. TOCTOU race on chain_state (single write lock throughout `produce_block`)
9. Unused chain_state lock in `submit_transaction` (removed)
10. Block signing conditional (made mandatory)
11. `build_runtime` random key each call (persists to `consensus.key`)
12. CLI temp directory usage (all commands use persistent `data-dir`)
13. `apply_batch` not atomic (WAL with `recover_pending_batches`)
14. `System::new_all()` performance (switched to `System::new()`)
15. Contract deploy overwriting sender wallet (removed destructive insert)
16. Block hash missing `gas_limit` and `priority` (added to hash computation)
17. `MerkleTree::root()` requiring `&mut self` (refactored to `&self` with `RefCell`)
18. Dead dependencies `proptest` and `quickcheck` (removed)

### 2.3 Remaining Gaps (31 Items — Full Audit)

#### CRITICAL (1 item)

| # | Gap | Spec Ref | Source | Impact |
|---|-----|----------|--------|--------|
| C7 | No TLS for P2P communications | `Production_Readiness_Guide.md:54-58` | `sync.rs` uses raw `TcpStream` | All network traffic in plaintext; trivial MITM |

#### HIGH (10 items)

| # | Gap | Spec Ref | Source |
|---|-----|----------|--------|
| H1 | Nonce-gap transactions rejected instead of queued | `Transaction_Mempool.md:152-153` | `runtime.rs:386-393` |
| H2 | Mempool eviction missing "least gas" tier | `Transaction_Mempool.md:170-182` | `runtime.rs:141-159` |
| H3 | Float/non-deterministic WASM opcodes not banned | `Smart_Contract_Module.md:38`, `Core_Engine_Runtime.md:237` | `contracts.rs:981-1007` |
| H4 | Deep WASM validation missing (memory pages, export/import whitelist) | `Smart_Contract_Module.md:34-43` | `contracts.rs:990-1007` |
| H5 | Sled config (`cache_size_mb`, compression) not applied | `Production_Readiness_Guide.md:86-95` | `config.rs:39-44` defined, `storage.rs:178` ignores |
| H6 | No chain reorganization / fork resolution execution | `Consensus_PoA.md:128-138` | `sync.rs:252-260` detects but doesn't switch |
| H7 | Block `nonce` not validated by consensus | `Consensus_PoA.md:126` | `consensus.rs:64-100` |
| H8 | Gas estimation returns stub value (constant 5000) | `Smart_Contract_Module.md:152-170` | `contracts.rs:1196-1208` |
| H9 | Transaction history only scans mempool, not blockchain | `Core_Engine_Runtime.md:229` | `runtime.rs:625-647` |
| H10 | No capability-based WASI security model | `Smart_Contract_Module.md:150` | `contracts.rs` |

#### MEDIUM (11 items)

| # | Gap | Spec Ref |
|---|-----|----------|
| M1 | Mempool TTL 5 minutes vs spec's 24-hour example | `Transaction_Mempool.md:182` |
| M2 | `ContractEngine` trait signatures differ from spec (extra params, different return types) | `Core_Engine_Runtime.md:171-173` |
| M3 | No `WasmRuntime` sub-trait or `get_wasm_runtime()` method | `Core_Engine_Runtime.md:181-186` |
| M4 | No `cdylib` compilation target (needed for Go/JS FFI) | `Core_Engine_Runtime.md:243` |
| M5 | Storage key prefixes don't match schema (`"acc:"` and `"code:"` prefixes missing) | `Storage_Layer.md:128,134` |
| M6 | `get_transaction_by_id` returns `Transaction` not `(Block, Transaction)` | `Core_Engine_Runtime.md:142` |
| M7 | `compact()` only flushes trees — no actual space reclamation | `Production_Readiness_Guide.md:317-331` |
| M8 | Benchmark file has compilation errors (references non-existent fields) | `benches/performance_benchmarks.rs` |
| M9 | Contract event log not persisted or queryable | `Smart_Contract_Module.md:143-144` |
| M10 | Inter-contract call results lost between execution contexts | `Smart_Contract_Module.md:140` |
| M11 | `clear_pending_transactions` named `clear_mempool` in trait | `Core_Engine_Runtime.md:138` |

#### LOW (9 items)

| # | Gap | Spec Ref |
|---|-----|----------|
| L1 | Crate named `baals` not `libchain` | `Core_Engine_Runtime.md:243` |
| L2 | Hash fields use `[u8; 32]` not `String` | `Core_Engine_Runtime.md:91-94` |
| L3 | Metadata uses `BTreeMap<String, String>` not `Map<String, Value>` | `Core_Engine_Runtime.md:101,115` |
| L4 | `recipient` is `Address` enum not `PublicKey` | `Core_Engine_Runtime.md:107` |
| L5 | NodeJS SDK is TypeScript types only — no native addon | `CLI_SDK_Wiring_Overview.md:140-141` |
| L6 | `baals_storage_remove` inserts empty vec instead of deleting | `Smart_Contract_Module.md:114` |
| L7 | `Runtime::start()` doesn't begin block production loop | `CLI_SDK_Wiring_Overview.md:31` |
| L8 | No multi-validator PoA or alternate consensus plugins | `Consensus_PoA.md:154-168` |
| L9 | Keystore API takes `Option<PathBuf>` not required string | `Production_Readiness_Guide.md:25-31` |

---

## 3. Completion Roadmap

### Phases 1-9: Foundation ✓ COMPLETE (2026-05-05)

Phases 1-9 delivered the core engine, smart contracts, CLI, config, validation hardening, FFI/SDK layers, advanced ledger/consensus features, sync protocol, production hardening, and final spec compliance. Verified by audit: 64/72 claims confirmed, 3 Go SDK signatures fixed, 5 minor plan.md inaccuracies corrected.

### Phase 10: Critical Spec Gaps (Current — 1 item)

**Goal**: Fix the remaining critical gaps that prevent production deployment.
**Duration Estimate**: 1-2 weeks
**Specs**: All 9 spec documents

- [x] **C1: Automatic block production** (`src/runtime.rs`, `src/main.rs`)
  - `Runtime::start()` now spawns a background worker based on `PoAConsensus.block_time_interval_ms`
  - Added consensus accessor `block_time_interval_ms()` to the `ConsensusEngine` trait
  - Added mempool-threshold trigger (`mempool_size_limit / 2`, minimum 1) before automatic production
  - Added integration coverage: `test_runtime_auto_block_production_from_mempool_threshold`
- [x] **C2: Future block timestamp validation** (`src/ledger.rs`, `src/consensus.rs`)
  - Added wall-clock bound check in both ledger and consensus (`MAX_FUTURE_BLOCK_TIMESTAMP_SECONDS = 10`)
  - Added integration coverage: `test_ledger_rejects_future_block_timestamp`
- [x] **C3: Atomic block processing** (`src/ledger.rs`)
  - `apply_block` now fails fast on any transaction execution failure
  - Prevents partial state commits by aborting before batch apply
  - Added integration coverage: `test_block_application_is_atomic_on_transaction_failure`
  - Contract engine-internal rollback remains a separate known limitation
- [x] **C4: Contract storage Merkle tree** (`src/storage.rs`, `src/ledger.rs`)
  - Added `get_all_contract_storage_keys(contract_id) -> Vec<Vec<u8>>` to `Storage` trait and `SledStorage`
  - `apply_block` now recomputes Merkle roots from persisted contract storage key/value pairs
  - `storage_root_hash` is now written to the contract account key (deterministic contract-id mapping)
  - Added integration coverage: `test_contract_storage_root_tracks_contract_kv_state`
- [x] **C5: Sparse Merkle tree for account state** (`src/types.rs`, `src/ledger.rs`)
  - Implemented `SparseMerkleTree` with key-indexed leaves (key = account address hash, value = serialized account)
  - Wired sparse tree root into ledger `accounts_root_hash` calculation
  - Added sparse proof generation/verification primitives with unit coverage: `test_sparse_merkle_proof_roundtrip`
- [x] **C6: HTTP health endpoint** (`src/main.rs`, `src/runtime.rs`, `src/config.rs`)
  - Added lightweight `tiny_http` server exposing `GET /health`
  - Returns JSON `HealthStatus` via `Runtime::get_health_status()` built on `MetricsCollector::health_check()`
  - Added configurable `node.health_port` (default `8080`) and wired node-start lifecycle
  - Added integration coverage for runtime health shape: `test_runtime_health_status_exposes_chain_and_mempool`
- [ ] **C7: TLS for P2P** (`src/sync.rs`, `src/config.rs`)
  - Add TLS fields to `NetworkConfig` (`tls_enabled`, `cert_path`, `key_path`)
  - Upgrade `TcpStream` to `tokio_native_tls::TlsStream` when enabled
  - Add peer certificate verification and optional certificate pinning

### Phase 11: High Priority Gaps (10 items)

**Goal**: Fix the 10 high-severity spec gaps.
**Duration Estimate**: 1-2 weeks

- [ ] **H1: Nonce-gap transaction queuing** (`src/runtime.rs`) — accept txs with nonce > expected, store in gap queue, process when intermediate nonces arrive
- [ ] **H2: Complete mempool eviction tiers** (`src/runtime.rs`) — add least-gas-limit-first eviction as specified
- [ ] **H3: Float opcode banning** (`src/contracts.rs`) — scan WASM bytecode for non-deterministic opcodes (floats, `memory.grow` only allowed via host function) at deploy time
- [ ] **H4: Deep WASM validation** (`src/contracts.rs`) — validate memory page limits, require specific WASI imports, whitelist exports
- [ ] **H5: Apply sled config** (`src/storage.rs`) — pass `cache_capacity` and `compression` from `StorageConfig` to `sled::Config`
- [ ] **H6: Chain reorganization** (`src/sync.rs`, `src/runtime.rs`) — implement `reorganize_chain()` that switches to longer valid chain after fork detection
- [ ] **H7: Block nonce validation** (`src/consensus.rs`) — validate nonce is a monotonically increasing value or meets expected pattern
- [ ] **H8: Working gas estimation** (`src/contracts.rs`) — implement static analysis or dry-run execution to produce accurate estimates
- [ ] **H9: Blockchain-backed transaction history** (`src/runtime.rs`) — query storage `get_transactions_by_address()` instead of mempool-only
- [ ] **H10: Capability-based WASI security** (`src/contracts.rs`) — implement per-module permission grants for host functions

### Phase 12: Medium Priority Gaps (11 items)

**Goal**: Fix the 11 medium-severity spec gaps.
**Duration Estimate**: 1-2 weeks

- [ ] **M1: Configurable mempool TTL** (`src/runtime.rs`) — make `ttl_seconds` configurable (default 300s, but allow 24h)
- [ ] **M2: Align ContractEngine trait signatures** (`src/contracts.rs`) — match spec or document divergences
- [ ] **M3: WasmRuntime sub-trait** (`src/contracts.rs`) — extract WASM instantiation/execution into separate trait
- [ ] **M4: cdylib compilation target** (`Cargo.toml`) — add `[lib] crate-type = ["cdylib"]` for FFI consumers
- [ ] **M5: Storage key prefix schema** (`src/storage.rs`) — add `"acc:"`, `"code:"`, etc. prefixes per spec
- [ ] **M6: Fix get_transaction_by_id return type** (`src/storage.rs`) — return `(Block, Transaction)` pair
- [ ] **M7: Real storage compaction** (`src/storage.rs`) — use sled's compaction API if available, or full export/import cycle
- [ ] **M8: Fix benchmark compilation** (`benches/performance_benchmarks.rs`) — update to match current structs
- [ ] **M9: Persist contract events** (`src/contracts.rs`, `src/storage.rs`) — store events to sled, add query API
- [ ] **M10: Persist inter-contract results** (`src/contracts.rs`) — store results in storage for cross-call-context access
- [ ] **M11: Rename clear_mempool → clear_pending_transactions** (`src/storage.rs`) — match spec naming

### Phase 13: Maintenance & Low Priority (9 items)

**Goal**: Clean up technical debt and low-severity mismatches.
**Duration Estimate**: 1 week

- [ ] **L1-L4**: Document intentional design deviations (byte hash arrays, Address enum, metadata types) in spec compliance notes
- [ ] **L5: NodeJS native addon** (`sdk/nodejs/`) — implement via `napi-rs` or `neon`
- [ ] **L6: Fix baals_storage_remove** (`src/contracts.rs`) — delete key instead of inserting empty vec
- [ ] **L7-L9**: Document that `start()` behavior, single-validator PoA, and keystore API shape are intentional for MVP

### Phase 14: Testing & Quality Hardening

**Goal**: Expand test coverage for untested critical paths.
**Duration Estimate**: 1-2 weeks

- [ ] Add sync protocol integration tests (handshake, block download, fork detection)
- [ ] Add consensus signing verification tests (valid vs invalid signature, wrong signer)
- [ ] Add keystore encryption round-trip tests
- [ ] Add reentrancy guard tests (deploy contract that calls itself)
- [ ] Add inter-contract call tests (deploy two contracts, one calls the other)
- [ ] Add negative-path tests for all validation functions (invalid hashes, bad nonces, insufficient balance)
- [ ] Add concurrency tests (multiple simultaneous `submit_transaction` + `produce_block`)
- [ ] Fix and run `benches/performance_benchmarks.rs`
- [ ] Run `cargo-tarpaulin` for coverage report; target >80% line coverage

### Phase 15: Dependency & Infrastructure Updates

**Goal**: Modernize dependencies and infrastructure.
**Duration Estimate**: 1 week

- [ ] Upgrade `wasmtime` from 18.0 → 27+ (review API changes for `consume_fuel`, `Config`)
- [ ] Upgrade `rand` 0.8 → 0.9, `sysinfo` 0.29 → 0.33
- [ ] Evaluate `sled` → `redb` or `rocksdb` migration path (sled maintainer considers project feature-complete)
- [ ] Add CI/CD pipeline (GitHub Actions: build, test, lint, coverage, Docker publish)
- [ ] Add `rustfmt.toml` and `clippy.toml` configuration
- [ ] Add `.github/dependabot.yml` for automated dependency updates

---

## 4. Milestones & Deliverables

| Milestone | Status | Deliverable |
|---|---|---|
| Phase 1: Foundation Hardening | ✅ Done | Bug-free core engine, serialization fixes, mempool correctness |
| Phase 2: Smart Contract Runtime | ✅ Done | 15 WASM host functions, gas metering, contract deploy/execute |
| Phase 3: CLI & Config System | ✅ Done | 25+ CLI subcommands, TOML config, --json flag |
| Phase 4: Transaction & Mempool Hardening | ✅ Done | Timestamp/gas/payload validation, eviction, TTL, rate limiting |
| Phase 5: FFI & Multi-Language SDKs | ✅ Done | C ABI (13 exports), Go SDK, TypeScript definitions |
| Phase 6: Advanced Ledger & Consensus | ✅ Done | Block signing, Merkle roots, accounts_root_hash |
| Phase 7: Sync Layer | ✅ Done | TCP P2P, 16 message types, fork detection |
| Phase 8: Production Hardening | ✅ Done | 26 tests, zero warnings, plan updated |
| Phase 9: Final Spec Compliance | ✅ Done | Audit fixes, Dockerfile, K8s, JSON logging, bounds-checking |
| Phase 10: Critical Spec Gaps | ⬜ Pending | TLS |
| Phase 11: High Priority Gaps | ⬜ Pending | Nonce-gap queuing, eviction tiers, float opcode ban, deep WASM validation, sled config, chain reorg, nonce check, gas estimation, tx history, capability security |
| Phase 12: Medium Priority Gaps | ⬜ Pending | TTL config, trait alignment, WasmRuntime subtrait, cdylib, key prefixes, return types, compaction, benchmarks, events, inter-contract results, naming |
| Phase 13: Low Priority & Maintenance | ⬜ Pending | Design doc updates, NodeJS addon, storage_remove fix, documentation |
| Phase 14: Testing & Quality | ⬜ Pending | Sync/consensus/keystore/reentrancy tests, negative paths, coverage |
| Phase 15: Dependency & Infrastructure | ⬜ Pending | wasmtime upgrade, rand/sysinfo updates, sled evaluation, CI/CD |

---

## 5. Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| wasmtime 18.0 CVEs | High | Upgrade to 27+ in Phase 15; pin version with `=18.0` in interim |
| sled maintenance status | Medium | `Storage` trait abstraction enables migration; evaluate `redb` in Phase 15 |
| No TLS for P2P | High | Plaintext MITM; implement TLS in Phase 10 (C7) |
| Go SDK needs compiled cdylib to link | Medium | Add `crate-type = ["cdylib"]` and CI build in Phase 12/15 |
| NodeJS SDK types-only | Low | Implement native addon in Phase 13 (L5) |
| Benchmarks don't compile | Low | Fix in Phase 14 |
| Single-validator PoA (no rotation) | Low | Document as MVP limitation; multi-validator in future phase |

---

## 6. Dependencies & Prerequisites

**Build**:
- Rust 1.70+ (stable toolchain)
- `wasmtime` system dependencies (LLVM/Clang for some platforms)

**SDK Development**:
- Go 1.21+ (for Go SDK)
- Node.js 18+ with `node-gyp` or `napi-rs` (for JS SDK)

**Testing**:
- `cargo-nextest` (faster test runs)
- `cargo-tarpaulin` (coverage reporting)

**Phase 10+ New Dependencies**:
- `tiny_http` or `axum` (HTTP health endpoint, C6)
- `tokio-native-tls` or `rustls` (TLS, C7)

---

## 7. Notes

- **No stubs without approval**: If a feature cannot be fully implemented within a phase, it must be explicitly scoped out rather than committed as a stub.
- **Spec-driven development**: Every PR must reference the governing spec document and the relevant checklist item in this plan.
- **Test-driven for new features**: New functionality requires a failing test before implementation, and a passing test before merge.
- **Design deviations**: Where the implementation intentionally differs from spec (hash fields as `[u8; 32]` vs `String`, `Address` enum vs `PublicKey`), these are documented in Phase 13 (L1-L4) as intentional improvements for type safety and performance.
