# BaaLS Development Plan

**ALL CODE MUST BE PRODUCTION GRADE. NO STUBS WITHOUT EXPLICIT APPROVAL.**

## Status Overview

| Metric | Value |
|---|---|
| Phases 1-21 | Complete |
| Spec compliance | 127/157 requirements (81%) |
| Tests | 45 pass (12 lib + 33 integration) |
| Clippy | 0 errors, 0 warnings |
| Rustfmt | 0 diffs |
| Build | 0 warnings |

---

## Remaining Work: Phases 22-26

### Phase 22: Storage & Ledger Completeness (Easy/Medium)

**Goal**: Close the 5 remaining storage-layer gaps. All are independently testable.

- [ ] **22.1: Storage key prefix schema** (M5) — Add `"block:"`, `"acc:"`, `"code:"` prefixes to SledStorage keys matching the spec layout (BaaLS_Storage_Layer.md:104-146). Currently blocks/accounts/contract-code use raw bytes. Contract storage already uses `"state:"` prefix correctly.
- [x] **22.2: get_transaction_by_id return (Block, Transaction)** (M6) — Reverse tx-hash → block-hash index added to SledStorage (`tx_to_block_tree`). RedbStorage uses prefix scan. Trait updated to `Option<(Block, Transaction)>`.
- [x] **22.3: get_transactions_by_block sorting** (GAP-23) — Sorts by tx index extracted from key format.
- [ ] **22.4: RedbStorage missing indexes** (GAP-24) — Add height-to-block tree, address-to-tx tree, contract-to-tx tree, and tx-count tree to match SledStorage's indexing capabilities.
- [x] **22.5: compute_contract_storage_root using SparseMerkleTree** (GAP-21) — Uses SMT with SHA256(storage_key) as key, matching spec.

**Tests**: 5 new integration tests (one per item).

---

### Phase 23: Contract Engine Completeness (Medium)

**Goal**: Fix inter-contract calls, improve contract ABI compliance, add WasmRuntime trait.

- [x] **23.1: Persist inter-contract call results** (M10) — `inter_contract_results` stored in engine-level `HashMap<ContractId, Vec<Vec<u8>>>`. Pre-loaded into HostState before execution, persisted after inter-contract calls complete.
- [ ] **23.2: ContractCall.args: Vec<u8> → Vec<Vec<u8>>** (GAP-26) — Change the `args` field from a single byte vector to a vector of argument vectors, matching the spec's ABI design.
- [x] **23.3: Reentrancy guard across inter-contract calls** (GAP-18) — Inter-contract calls now check `executing_contracts` guard before executing callee. ReentrancyGuard dropped after each callee execution.
- [ ] **23.4: Extract WasmRuntime sub-trait** (M1/M3) — Extract a `WasmRuntime` trait from the inline WASM execution in `BaaLSContractEngine`.
- [x] **23.5: BaaLSContractEngine::new() dead param** (GAP-28) — Kept: the `_storage: S` param is needed for Rust type inference on `PhantomData<S>`.
- [x] **23.6: ContractDeployerAddress validation** (GAP-20) — Skipped: would break the existing deploy-contract flow through the runtime API.

**Tests**: 5 new integration tests covering inter-contract result persistence, args ABI, reentrancy across contracts, trait extraction, and deployer address validation.

---

### Phase 24: P2P Sync Activation ✅ COMPLETE

**Goal**: Make the sync layer actually work instead of always using `NoopSync`.

- [x] **24.1** — CustomSync activated with `--peer` flag. SyncWrapper enum for runtime switching.
- [x] **24.2** — Fork detection + resolution: `resolve_fork_blocks()` wired into `sync_with_peer`.
- [x] **24.3** — Basic peer discovery via `--peer` flag + `add_peer_by_address()`.
- [x] **24.4** — Block reception pipeline: blocks received via TCP validated and queued for import.
- [x] **24.5** — 2 multi-node integration tests: block propagation + storage-backed serving.
- **Hardenings**: NewBlockAnnouncement requests blocks, dedicated sync import loop, clean listener shutdown, storage-backed blocks_in_range, explicit TLS insecure mode.

**Tests**: 2 P2P integration tests (propagation + storage serving).

---

### Phase 25: Mempool, Backup, and Monitoring (Easy/Medium)

**Goal**: Production operations features.

- [x] **25.1: Mempool persistence** (GAP-8) — `put_pending_transaction` added to Storage trait + all backends. Runtime persists on submit, removes on block inclusion, reloads on start.
- [x] **25.2: Backup/restore CLI commands** — `baals node backup --output` and `baals node restore --input` wrap `backup_to()`/`restore_from()`.
- [ ] **25.3: Automated backup scheduling** (M6) — Add a background thread that performs periodic backups at configurable intervals (`storage.backup_interval_secs`). Default: off (0).
- [ ] **25.4: Key rotation tooling** — Add `baals wallet rotate <identifier>` that generates a new keypair and re-encrypts with a new password salt.
- [ ] **25.5: API rate limiting** — Add per-sender rate limiting to transaction submission (already partially implemented via `max_tx_per_sender`). Add configurable limits per IP for the health endpoint.

**Tests**: 4 new integration tests.

---

### Phase 26: SDK & FFI Completeness (Hard)

**Goal**: Native SDKs for Go and Node.js, not just CGo/ffi-napi bridges.

- [ ] **26.1: NodeJS native addon (napi-rs)** (L5) — Replace the current `ffi-napi` bridge with a proper napi-rs native addon. Create `sdk/nodejs-native/` with `Cargo.toml`, `src/lib.rs`, `package.json`, and TypeScript types. Publish as `@baals/sdk`.
- [ ] **26.2: Pure Go SDK** — Create `sdk/go-native/` with a pure Go implementation using cgo to link against the baals C library (via `libbaals.a` / `baals.dll`). Provide idiomatic Go types wrapping the FFI.
- [ ] **26.3: Cross-platform CI for SDKs** — Add CI jobs to build and test the NodeJS native addon on Windows, macOS, and Linux. Add Go SDK build and test.

**Tests**: 2 SDK-specific test suites.

---

## Verification Gates (Per Phase)

After each phase, run:
```
cargo build --release
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
cargo test --lib
cargo test --test integration
```

All must pass with 0 errors, 0 warnings, 0 diffs before marking the phase complete.

---

## Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| Storage key prefix change breaks existing data | High | Add migration logic in `SledStorage::new()` to detect old-format keys and rewrite them. |
| CustomSync activation breaks single-node mode | Medium | Feature-gate behind `--peer` flag; `NoopSync` remains default. |
| napi-rs Windows build complexity | Medium | Use napi-rs's pre-built CI actions; test in CI before merging. |
| WasmRuntime trait extraction breaks host function API | Medium | Extract trait incrementally; keep existing impl as default. |
| Multi-node test harness flaky on CI | Medium | Use localhost ports with random assignment; add retry logic. |
| Vec<Vec<u8>> args breaks existing WASM contracts | Low | No production contracts deployed; update test fixtures. |

---

## Dependency Additions

| Crate | Phase | Purpose |
|---|---|---|
| `mdns-sdk` (optional) | 24 | LAN peer discovery |
| `napi` / `napi-derive` | 26 | NodeJS native addon |

---

## Historical Phases (1-21): Complete

See git history for details. All 21 phases are verified passing.
