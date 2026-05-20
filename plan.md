# BaaLS Production Plan — Agentic Gap Closure Roadmap

**Last updated:** 2026-05-19
**Baseline:** All Phase 1 bug fixes (CQ-1–CQ-17), Phase 2 surface items (finality, supply, metrics, JWT, TLS, checksums), and 2026-05-19 remediation tasks (T0.1–T4.1) are **COMPLETE**. However, `src/contracts.rs` has a broken partial refactor that prevents compilation — see Phase 0.
**Target:** Close every remaining gap between spec docs and shipped code.

---

## Current State Summary

| Area | Status | Notes |
|------|--------|-------|
| Phase 1 bugs (CQ-1–17) | ✅ Fixed | Regression suite 11/11 |
| Phase 2 surface items | ✅ Shipped | Finality, supply, metrics, JWT, TLS, checksums |
| Remediation T0–T4 | ✅ Done | Quality gates, API namespace, stubs removed, docs reconciled |
| Multi-validator PoA | ⚠️ Partial | Signer set + persistence exist; no quorum, round-robin, or voting |
| P2P CLI commands | ❌ Not wired | Return explicit `not_implemented` |
| WebSocket API | ❌ Not implemented | REST only; `tiny_http` is synchronous (can't WS upgrade) |
| Contract engine estimate-gas | ✅ Engine impl | `estimate_gas_usage()` does real dry-run; CLI command is stubbed |
| Contract ABI storage/retrieval | ❌ Stubbed | CLI returns not-implemented |
| Admin rotate-consensus-key / tls-generate | ❌ Stubbed | Explicitly not-implemented |
| Rollback logs / snapshots | ❌ Not implemented | Divergent forks fail closed |
| Inter-contract call results | ❌ **BROKEN** | `call_results` computed but never stored; `baals_read_call_result` can't read them |
| Block hash→sign pipeline | ⚠️ Inverted | Signer set in metadata AFTER hash; hash doesn't cover signer identity |
| Block struct typed fields | ⚠️ Missing | No `total_gas_used` field; no typed signature fields; relies on `BTreeMap<String,String>` metadata |
| Missing REST endpoints | ❌ Not implemented | tx-by-hash, tx-by-address, contract state, peers, sync |
| Python SDK | ❌ Planned | Not in repo |
| Mobile SDKs | ❌ Roadmap | Not in repo |
| RocksDB backend | ❌ Mentioned in spec | sled + redb only |
| PoS / PoW / CRDT plugins | ❌ Future | PoA only |
| `contracts.rs` compilation | ❌ **BROKEN** | `HostState::charge_gas` removed but 8 call sites remain |
| Rust SDK (`sdk.rs`) | ⚠️ Hardcoded | Locks to `SledStorage` + `NoopSync`; no redb or real sync |
| FFI layer (`ffi.rs`) | ⚠️ Partial | Wraps SDK; inherits SDK limitations |
| Go SDK (`sdk/go/`) | ⚠️ Untested | HTTP client exists; no integration test coverage |
| Node.js SDK (`sdk/nodejs-native/`) | ⚠️ Untested | napi-rs bindings exist; no integration test coverage |

---

## Execution Order

Tasks are ordered by dependency — critical path first. The dependency chain is:

```
Phase 0 (compilation) → Phase 0A (architecture) → Phase A (quorum) ──→ Phase G.1/G.4 (tests)
                                                 → Phase B (rollback) → Phase G.2/G.3 (tests)
                                                 → Phase C–F (parallel-safe)
                                                 → Phase I (SDK) → Phase J.4 (SDK tests)
                                                 → Phase J (spec gaps, mostly independent)
                                                 → Phase H (docs — do last)
```

Each task has:
- **Acceptance criteria** — measurable, testable conditions
- **Files to touch** — exact source paths
- **Agent instructions** — what an autonomous agent should do
- **Verification** — commands to run after implementation

---

## Phase 0 — Fix Compilation (BLOCKING)

**Why first:** The project does not compile. Nothing else can proceed until this is resolved.

### 0.1 Complete `HostState` Gas Metering Migration

**Problem:** `src/contracts.rs` has a half-finished refactor. The `HostState` struct's `gas_used` and `gas_limit` fields were removed, along with the `impl HostState` block containing `charge_gas()` and `charge_memory_growth()`. However, **8 call sites** still invoke these deleted methods (lines 621, 681, 717, 814, 871, 939, 974, 1023). A replacement helper `consume_host_fuel()` was added (using wasmtime's built-in fuel metering) but never wired in.

**Agent instructions:**
1. In each host function inside `add_host_functions()`, replace every `state.charge_gas(N)` call with a call to `consume_host_fuel(&mut caller, N)`. The helper already exists at the top of `add_host_functions`.
2. Replace `state.charge_memory_growth(current_size)` in `baals_memory_grow` with equivalent fuel-based logic: compute growth pages from `last_memory_size` delta, call `consume_host_fuel` with `pages * 100`.
3. Track `last_memory_size` updates — since `HostState` still has the field, update it after charging.
4. Remove the now-unused `consume_host_fuel` inner function that was duplicated inside `add_host_functions` — there are two copies (one at the top-level scope of the function, one nested). Keep only the one that is reachable from all host function closures.
5. Verify all 8 error sites compile and that fuel exhaustion returns appropriate errors to the WASM caller (return -1 or equivalent).

**Files:** `src/contracts.rs`

**Acceptance criteria:**
- `cargo check` passes with zero errors.
- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all-features` passes (all existing tests).
- Gas metering still works: a contract that exceeds gas limit is terminated.

**Verification:**
```bash
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

---

## Phase 0A — Architectural Prerequisites (BLOCKING)

**Why before everything else:** These are structural defects in the core data model and execution pipeline. Later phases (A, E, F, G) cannot be implemented correctly on top of a broken foundation. Fix these first so every subsequent phase builds on sound architecture.

### 0A.1 Fix Block Hash→Sign Pipeline

**Problem:** In `consensus.rs::generate_block()`, the signer identity is added to `block.metadata` AFTER `block.calculate_hash()` is called. This means the block hash does not cover who signed it — an attacker could swap the signer field without invalidating the hash. This also prevents F.1 from including signer in the hash, and prevents A.1 from storing quorum signatures in a hash-committed way.

**Root cause:** `generate_block()` calls `calculate_hash()` then `sign_block()` which inserts signer/signature into metadata.

**Agent instructions:**
1. In `consensus.rs::generate_block()`, restructure the flow to:
   a. Set `block.metadata["signer"]` = signer public key hex BEFORE calling `calculate_hash()`.
   b. Call `block.calculate_hash()` (hash now covers signer identity).
   c. Sign the resulting hash with the signer's private key.
   d. Store signature in `block.metadata["signature"]` (signature is over the hash, not part of it — this is correct).
2. In `consensus.rs::validate_block()`, verify that:
   a. `metadata["signer"]` is present and is an authorized signer.
   b. Recalculated hash matches `block.hash` (signer is now included in the recalculation).
   c. `metadata["signature"]` is a valid Ed25519 signature of `block.hash` by `metadata["signer"]`.
3. Update `Block::calculate_hash()` in `types.rs` to include the `metadata["signer"]` value in the hash input (append after tx merkle root, before finalization). Only include signer — do NOT include signature (signature is computed over the hash).
4. **Migration:** Existing blocks used the old hash format. For now this is acceptable — this is a development chain, not a production chain with existing data that must be preserved. If backward compatibility is needed later, F.1's `hash_version` approach handles it.

**Files:** `src/consensus.rs` (generate_block, validate_block, sign_block), `src/types.rs` (Block::calculate_hash)

**Acceptance criteria:**
- Block hash changes when signer changes (same block data, different signer → different hash).
- `validate_block` rejects blocks where signer was swapped after hashing.
- All existing tests pass (update test fixtures for new hash values).
- `cargo test --all-features` passes.

**Verification:**
```bash
cargo test block --all-features
cargo test consensus --all-features
```

---

### 0A.2 Fix Inter-Contract Call Results Bug

**Problem:** In `contracts.rs`, the `execute_contract_call()` method computes `call_results: Vec<ContractCallResult>` for inter-contract calls (lines ~504-553), but **never stores them back** into `HostState` or any accessible location. The host function `baals_read_call_result` reads from `state.call_results`, which remains empty. This means contracts that make inter-contract calls cannot read the results — the feature is silently broken.

**Agent instructions:**
1. After the inter-contract call loop completes and `call_results` is populated, store it back into `HostState.call_results` (or wherever `baals_read_call_result` reads from).
2. Verify the index-based access in `baals_read_call_result` matches the order results are stored.
3. Add a test: contract A calls contract B, reads call result → gets correct return data.

**Files:** `src/contracts.rs`

**Acceptance criteria:**
- Inter-contract call results are readable by the calling contract.
- Test: deploy two contracts, A calls B via `baals_call_contract`, then reads result via `baals_read_call_result` → returns B's output.
- `cargo test --all-features` passes.

**Verification:**
```bash
cargo test contract --all-features
cargo test call_result --all-features
```

---

### 0A.3 Add Typed Fields to Block Struct

**Problem:** The `Block` struct relies on `BTreeMap<String, String>` metadata for critical consensus data (signer, signature). This works for single-signer PoA but cannot cleanly represent quorum signatures (A.1 needs `Vec<(PublicKey, Signature)>`), total gas used (F.1), or any structured data. String-encoding arrays of signatures into a `BTreeMap<String, String>` is fragile and error-prone.

**Agent instructions:**
1. Add typed fields to the `Block` struct in `types.rs`:
   - `total_gas_used: u64` (default 0, populated during `apply_block`).
   - `signer: Option<String>` (hex-encoded public key of primary block producer).
   - `signature: Option<Vec<u8>>` (Ed25519 signature bytes).
   - `quorum_signatures: Vec<(String, Vec<u8>)>` (for A.1; empty until quorum is implemented).
2. Update `Block::calculate_hash()` to include `total_gas_used` and `signer` (if present) in hash input.
3. Update `generate_block()` to populate `signer` and `signature` typed fields instead of (or in addition to) metadata. Keep metadata population for backward compat during transition.
4. Update `validate_block()` to read from typed fields.
5. Update `apply_block()` in `ledger.rs` to set `block.total_gas_used` as the sum of gas used by all transactions.
6. Ensure serialization/deserialization handles new fields with defaults (serde `#[serde(default)]`).

**Files:** `src/types.rs` (Block struct, calculate_hash), `src/consensus.rs` (generate_block, validate_block), `src/ledger.rs` (apply_block — set total_gas_used)

**Acceptance criteria:**
- `Block` struct has `total_gas_used`, `signer`, `signature`, `quorum_signatures` fields.
- `total_gas_used` is correctly populated after block application.
- Deserialization of old blocks (without new fields) succeeds with defaults.
- `cargo test --all-features` passes.

**Verification:**
```bash
cargo test --all-features
```

---

### 0A.4 Document Spec Deviations

**Problem:** The codebase has several intentional deviations from specs that are not documented:
- `Transaction` has a `gas_price` field; spec says "no fees" in local mode.
- Failed transactions still increment nonce and deduct gas (spec implies atomic rollback).
- `Account::Contract` has no `balance` field despite spec mentioning contract balances.
- Metadata uses `BTreeMap<String, String>` instead of spec's `Option<Map<String, Value>>`.

**Agent instructions:**
1. In `docs/Spec_Compliance_Notes.md`, add a "Known Deviations" section with a table:
   - `gas_price` field: present for future use, currently set to 0 in local mode. Document as L3 deviation — reserves field for fee markets.
   - Failed tx gas deduction: document as intentional spam-prevention measure even in PoA mode.
   - `Account::Contract` balance: not implemented; contracts hold no native balance. Document as L3 deviation — deferred to token standard phase.
   - Metadata type: `BTreeMap<String, String>` for deterministic ordering. Already noted as F.2; cross-reference.

**Files:** `docs/Spec_Compliance_Notes.md`

**Acceptance criteria:**
- Every known spec deviation is documented with rationale and severity level.
- No surprises when reading code vs. spec.

---

## Phase A — Multi-Validator PoA with Quorum

**Why next:** Single-signer PoA is a SPOF. With architectural prerequisites fixed (typed signature fields, correct hash→sign pipeline), quorum implementation can build on a solid foundation.

### A.1 Quorum Threshold Validation

**Problem:** `PoAConsensus::validate_block` accepts ANY authorized signer. No minimum signature count.

**Agent instructions:**
1. Add `quorum_threshold: usize` to `PoAConsensus` (default = 1 for backward compat).
2. Add `quorum_signatures: Vec<(PublicKey, Vec<u8>)>` to `Block.metadata` (or a new field).
3. In `validate_block`, count valid signatures against `quorum_threshold`. Reject if count < threshold.
4. In `generate_block`, collect `M` signatures from available signers before emitting block.
5. Add `ValidatorSet` struct to `types.rs` with `signers: Vec<PublicKey>`, `quorum: usize`, `effective_height: u64`.
6. Persist `ValidatorSet` to storage, load at startup.

**Files:** `src/consensus.rs`, `src/types.rs`, `src/storage.rs`, `src/ledger.rs`, `src/config.rs` (add `consensus.quorum_threshold` config field), `src/runtime.rs` (wire quorum config into consensus init)

**Acceptance criteria:**
- Block with < M valid signatures is rejected by `validate_block`.
- `cargo test` includes `test_quorum_rejection` and `test_quorum_acceptance`.
- Config `consensus.quorum_threshold` controls M.

**Verification:**
```bash
cargo test quorum --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

---

### A.2 Round-Robin Signer Scheduling

**Problem:** Any authorized signer can produce blocks at any time. No ordering prevents collisions.

**Agent instructions:**
1. Add `last_signer_index: usize` to `PoAConsensus`.
2. In `generate_block`, select signer by `(block_index % authorized_signers.len())`.
3. Reject blocks signed by non-scheduled signer (with grace window of 1 block for liveness).
4. Log signer rotation events.

**Files:** `src/consensus.rs`

**Acceptance criteria:**
- Test with 3 signers: blocks 1,4,7 signed by signer[0]; 2,5,8 by signer[1]; 3,6,9 by signer[2].
- Out-of-order signer produces block → rejected after grace window.

**Verification:**
```bash
cargo test round_robin --all-features
```

---

### A.3 Validator Set Change Transactions

**Problem:** No on-chain mechanism to add/remove validators.

**Agent instructions:**
1. Add `TransactionPayload::ValidatorSetChange { added: Vec<PublicKey>, removed: Vec<PublicKey>, effective_height: u64 }`.
2. In `ledger.rs::apply_block`, when this payload is detected, queue the change to take effect at `effective_height`.
3. At block height == `effective_height`, update `ValidatorSet` in storage.
4. Require quorum of current validators to sign the change transaction (verify in `submit_transaction`).

**Files:** `src/types.rs` (add `ValidatorSetChange` variant to `TransactionPayload`), `src/ledger.rs` (apply validator changes during `apply_block`), `src/runtime.rs` (wire pending changes into block production and mempool validation)

**Acceptance criteria:**
- Integration test: submit validator change tx → wait for effective height → new validator produces block.
- Change tx signed by < quorum → rejected.

**Verification:**
```bash
cargo test validator_set --all-features
```

---

## Phase B — Rollback Logs / Snapshots for True Reorg

**Why second:** Divergent forks currently fail closed. This is the biggest protocol gap.

### B.1 Write-Ahead Rollback Log

**Problem:** `reorganize_chain` rejects divergent forks because there's no way to undo applied blocks.

**Agent instructions:**
1. Create `src/rollback.rs` with `RollbackLog` struct.
2. For each `StorageOperation` applied in `apply_block`, generate an inverse operation:
   - `PutAccount(k, v)` → inverse needs old value; capture it before write.
   - `PutTransaction(k, v)` → inverse: `DeleteTransaction(k)`.
   - `PutContractStorage(k, v)` → inverse needs old value or `DeleteContractStorage(k)`.
   - `DeleteX(k)` → inverse needs old value to restore.
3. Store rollback log per block: `block_hash -> Vec<InverseOperation>`.
4. On reorg: walk from local tip to common ancestor, replay inverse ops in reverse order.
5. After rollback, apply fork blocks forward.

**Files:** `src/rollback.rs` (new), `src/ledger.rs`, `src/storage.rs`, `src/runtime.rs` (`reorganize_chain()` at line ~1123 is the primary integration point — currently rejects divergent forks)

**Acceptance criteria:**
- Test: apply 3 blocks, reorg to fork at block 1 → state matches fork tip.
- Test: rollback + reapply produces same state root as original.
- `db rollback-info` command shows rollback log depth.

**Verification:**
```bash
cargo test rollback --all-features
cargo test reorg --all-features
```

---

### B.2 Snapshot-Based Recovery (Fallback)

**Agent instructions:**
1. Add `take_snapshot(block_height)` to `Storage` trait.
2. Snapshot = copy of all account key-values + chain state at a point in time.
3. Keep last 3 snapshots (configurable).
4. On reorg deeper than rollback log capacity, restore from nearest snapshot and replay forward.

**Files:** `src/storage.rs` (trait definition + sled impl), `src/redb_storage.rs` (redb impl)

**Acceptance criteria:**
- `db snapshot` command creates snapshot.
- `db restore --snapshot <height>` restores state.
- Test: kill node mid-block, restore from snapshot, verify state consistency.

**Verification:**
```bash
cargo test snapshot --all-features
```

---

## Phase C — P2P CLI Commands Wired to Runtime

### C.1 Wire `p2p peers` to Live State

**Agent instructions:**
1. `p2p peers` should query the running node's HTTP API (`GET /api/v1/peers`) or connect to the node's sync layer.
2. Return actual peer list from `CustomSync::known_peers`.
3. If node not running, return error with instructions to start node.

**Files:** `src/cli/p2p.rs`, `src/cli/node.rs` (add `/api/v1/peers` endpoint), `src/sync.rs` (expose `CustomSync::known_peers` and peer state)

**Acceptance criteria:**
- `p2p peers --data-dir ./data` returns live peer list when node is running.
- Returns non-zero exit + "node not running" when node is stopped.

---

### C.2 Wire `p2p add-peer` / `remove-peer` / `sync-now`

**Agent instructions:**
1. `p2p add-peer <addr>` → POST to `/api/v1/peers` with address.
2. `p2p remove-peer <addr>` → DELETE `/api/v1/peers/<addr>`.
3. `p2p sync-now` → POST `/api/v1/sync/trigger`.
4. Add corresponding HTTP endpoints in `cli/node.rs`.
5. Endpoints call `runtime.sync_layer().add_peer_by_address()` etc.

**Files:** `src/cli/p2p.rs`, `src/cli/node.rs` (add REST endpoints), `src/sync.rs` (add `add_peer_by_address`, `remove_peer`, `trigger_sync` methods to `CustomSync`)

**Acceptance criteria:**
- `p2p add-peer 127.0.0.1:9070` → peer appears in `p2p peers`.
- `p2p sync-now` → triggers sync tick, logs sync activity.

---

## Phase D — Contract / Admin CLI Implementation

### D.1 `contract estimate-gas` (Wire CLI to Engine)

**Note:** The engine-level `estimate_gas_usage()` already performs a real dry-run with gas metering (`src/contracts.rs:1566`). This task wires the **CLI command and HTTP endpoint** to it.

**Agent instructions:**
1. Add `POST /api/v1/contracts/estimate-gas` endpoint in `src/cli/node.rs` that accepts `{ contract_id, method, args }`.
2. Endpoint opens storage, instantiates the contract engine, calls `estimate_gas_usage()`.
3. Return `{ gas_used: u64, gas_limit_recommended: u64, confidence_level: f64 }`.
4. Update `src/cli/contract.rs` `EstimateGas` handler to call the HTTP endpoint instead of returning `not_implemented`.

**Files:** `src/cli/contract.rs` (wire CLI to HTTP), `src/cli/node.rs` (add REST endpoint)

**Acceptance criteria:**
- `contract estimate-gas --contract-id <id> --method balance_of` returns gas estimate.
- Dry-run does not mutate state.

---

### D.2 `contract abi`

**Agent instructions:**
1. If contract was deployed with ABI metadata, store ABI alongside contract code.
2. `contract abi --contract-id <id>` retrieves and displays ABI.
3. If no ABI stored, return "ABI not available for this contract".

**Files:** `src/cli/contract.rs`, `src/storage.rs`, `src/contracts.rs`

**Acceptance criteria:**
- Deploy contract with `--abi abi.json` → `contract abi` returns it.
- Deploy without ABI → appropriate error message.

---

### D.3 `admin rotate-consensus-key`

**Agent instructions:**
1. Implement the full ceremony workflow:
   a. Generate new key (encrypted) at specified path.
   b. Create rotation transaction signed by current consensus key.
   c. Store pending rotation in config with effective height.
   d. On restart, load pending rotation and activate at effective height.
2. Add `POST /api/v1/admin/rotate-key` endpoint (JWT-protected).

**Files:** `src/cli/admin.rs`, `src/cli/node.rs`, `src/consensus.rs`, `src/keystore.rs`

**Acceptance criteria:**
- `admin rotate-consensus-key --data-dir ./data --new-key-path new.key` completes workflow.
- After effective height, new key produces blocks, old key rejected.

---

### D.4 `admin tls-generate`

**Agent instructions:**
1. Use `rcgen` crate (already a dependency) to generate self-signed TLS cert + key.
2. `admin tls-generate --output ./tls` creates `server.crt` and `server.key`.
3. Support `--cn`, `--org`, `--days`, `--san` flags.

**Files:** `src/cli/admin.rs`

**Acceptance criteria:**
- `admin tls-generate --output ./tls` produces valid cert + key.
- Generated cert works with `node start --tls-enabled`.

---

## Phase E — WebSocket Streaming API

**Prerequisite note:** `tiny_http` is a synchronous HTTP server and cannot perform WebSocket upgrades on the same port. The solution is to run a separate async WebSocket server on a dedicated port alongside the existing HTTP API. This avoids a risky rewrite of the HTTP layer while still delivering WebSocket support.

### E.1 WebSocket Server (Separate Port)

**Agent instructions:**
1. Add `tokio`, `tokio-tungstenite`, and `futures-util` dependencies.
2. Create `src/websocket.rs` with a standalone async WebSocket server:
   a. Spawns on a configurable port (default: 8081, config key `api.ws_port`).
   b. Accepts WebSocket connections at `ws://host:ws_port/`.
   c. Each connection can subscribe to event channels: `blocks`, `transactions`, `mempool`.
   d. Message format: `{ "type": "subscribe", "channel": "blocks" }` / `{ "type": "unsubscribe", "channel": "blocks" }`.
   e. Server pushes: `{ "type": "new_block", "data": {...} }`, `{ "type": "new_tx", "data": {...} }`.
3. In `cli/node.rs`, spawn the WebSocket server in a separate thread (using `tokio::runtime::Runtime`) alongside the existing `tiny_http` server.
4. Create a broadcast channel (`tokio::sync::broadcast`) that the runtime writes to when blocks/txs are produced. The WS server reads from this channel and fans out to subscribers.
5. The HTTP health endpoint at the existing port should include `"ws_port": 8081` in its response so clients discover the WS port.

**Files:** `src/websocket.rs` (new), `src/cli/node.rs` (spawn WS server, create broadcast channel), `src/config.rs` (add `api.ws_port`), `src/runtime.rs` (emit events to broadcast channel), `Cargo.toml` (add dependencies)

**Acceptance criteria:**
- Client connects to `ws://localhost:8081/`, subscribes to blocks.
- New block produced → client receives JSON message within 100ms.
- HTTP API on port 8080 and WS on port 8081 run simultaneously.
- `GET /api/v1/health` response includes `ws_port` field.
- Test: `cargo test websocket --all-features`.

**Verification:**
```bash
cargo test websocket --all-features
```

---

### E.2 Update Docs

**Agent instructions:**
1. Add WebSocket section to `docs/OPERATING.md`.
2. Update `docs/BaaLS_CLI_SDK_Wiring_Overview.md` with WS support tier = "Beta".
3. Add WS example to `docs/Spec_Compliance_Notes.md`.

**Files:** `docs/OPERATING.md`, `docs/BaaLS_CLI_SDK_Wiring_Overview.md`, `docs/Spec_Compliance_Notes.md`

---

## Phase F — Spec Alignment (Type Deviations)

### F.1 Block Hash Versioning for Production Chains

**Note:** Phase 0A.1 fixes the hash→sign pipeline and 0A.3 adds signer + total_gas_used to the hash. This task adds a **versioned upgrade mechanism** for production chains that already have blocks mined with the old hash format. Skip this task if there is no existing chain data to preserve.

**Problem:** If a production chain exists with blocks hashed under the old format, a hard cutover would invalidate historical blocks.

**Agent instructions:**
1. Add a `hash_version: u8` field to `Block` (default 2 for new blocks, 1 for pre-0A.1 blocks).
2. In `Block::calculate_hash`, branch on `hash_version`:
   - v1: legacy behavior (index, timestamp, prev_hash, state_root, nonce, tx Merkle root only).
   - v2: current behavior from 0A.1/0A.3 (includes signer + total_gas_used).
3. Add `consensus.hash_upgrade_height: u64` to `src/config.rs`. Blocks below this height use v1; at or above use v2.
4. In `validate_block`, accept both v1 and v2 based on the block's `hash_version`.
5. Document upgrade procedure in `CHANGELOG.md` and `docs/OPERATING.md`.

**Files:** `src/types.rs`, `src/consensus.rs`, `src/config.rs`

**Acceptance criteria:**
- v1 blocks before upgrade height still validate.
- v2 blocks at or above upgrade height use new hash format.
- `cargo test block_hash --all-features` passes.

---

### F.2 Metadata: Document Deviation from Spec

**Problem:** Spec says `Option<Map<String, Value>>`, code uses `Option<BTreeMap<String, String>>`.

**Agent instructions:**
1. This is an intentional design choice (deterministic ordering, lower complexity).
2. Document in `docs/Spec_Compliance_Notes.md` as "L3 deviation — intentional".
3. No code change needed. (Cross-referenced from 0A.4.)

**Files:** `docs/Spec_Compliance_Notes.md`

---

## Phase G — Testing & Hardening

**Dependencies:** G.1 and G.4 require Phase A (quorum, round-robin, validator changes). G.2 and G.3 require Phase B (rollback logs, snapshots). Do not start these tests until their prerequisite phases are complete.

### G.1 Multi-Validator Integration Tests

**Agent instructions:**
1. Create `tests/multi_validator.rs`.
2. Test scenarios:
   - 3-of-5 quorum: 3 signers produce valid block.
   - 2-of-5 signers: block rejected.
   - Round-robin ordering across 10 blocks.
   - Validator set change at effective height.
   - Byzantine signer produces invalid block → rejected.

**Files:** `tests/multi_validator.rs` (new)

**Acceptance criteria:**
- All 5 scenarios pass.
- `cargo test --test multi_validator --all-features` passes.

---

### G.2 Reorg Stress Tests

**Agent instructions:**
1. Create `tests/reorg_stress.rs`.
2. Test scenarios:
   - Reorg depth 1, 5, 10, 50 blocks.
   - Reorg beyond `max_reorg_depth` → rejected.
   - Crash mid-reorg → restart produces consistent state.
   - Two competing forks → longest chain wins.

**Files:** `tests/reorg_stress.rs` (new)

**Acceptance criteria:**
- All scenarios pass.
- `cargo test --test reorg_stress --all-features` passes.

---

### G.3 Crash Recovery Tests

**Agent instructions:**
1. Create `tests/crash_recovery.rs`.
2. Test scenarios:
   - Kill node during `apply_block` (inject panic) → restart, verify state.
   - Kill node during backup → verify partial backup not restorable.
   - Kill node during DB migration → verify migration idempotent.
   - Rollback log survives crash.

**Files:** `tests/crash_recovery.rs` (new)

**Acceptance criteria:**
- All scenarios pass.
- `cargo test --test crash_recovery --all-features` passes.

---

### G.4 Byzantine Fault Tests

**Agent instructions:**
1. Create `tests/byzantine.rs`.
2. Test scenarios:
   - 3-node network, one Byzantine peer sends invalid blocks → rejected.
   - Validator produces two conflicting blocks at same height (equivocation) → detected.
   - Peer disconnects mid-block-send → partial block discarded.
   - Eclipse attack simulation → authorized peer list prevents it.

**Files:** `tests/byzantine.rs` (new)

**Acceptance criteria:**
- All scenarios pass.
- `cargo test --test byzantine --all-features` passes.

---

## Phase H — Documentation Reconciliation

### H.1 Update All Spec Docs

**Agent instructions:**
1. Review every doc in `docs/` against current code.
2. Update status tables to reflect actual implementation state.
3. Remove any claims not backed by code + tests.
4. Add "Planned" section for items in this plan that are not yet implemented.

**Files:** All `docs/*.md`

**Acceptance criteria:**
- No doc claims a feature is implemented when it isn't.
- No contradictions between docs.
- `docs/Spec_Compliance_Notes.md` has a "Gap Closure Progress" section tracking this plan.

---

### H.2 Add Gap Closure Progress Tracker

**Agent instructions:**
1. Add a section to `docs/Spec_Compliance_Notes.md` tracking each task in this plan.
2. Format: `| Task | Status | PR | Date |`
3. Update as tasks are completed.

**Files:** `docs/Spec_Compliance_Notes.md`

---

## Phase I — SDK / FFI Hardening

**Why:** `sdk.rs` and `ffi.rs` are the foundation for all external language bindings (Python, mobile). They currently have hardcoded limitations that block downstream SDK work.

### I.1 Make `BaaLSSdk` Generic Over Storage Backend

**Problem:** `BaaLSSdk` in `src/sdk.rs` hardcodes `SledStorage` and `NoopSync`. This means the SDK cannot use redb (the other supported backend) and has no real sync capability.

**Agent instructions:**
1. Make `BaaLSSdk` generic over `S: Storage` or use `AnyStorage` (which already exists in `src/any_storage.rs`).
2. Accept a `StorageBackend` enum from config to select sled vs redb at construction time.
3. Replace `NoopSync` with an option to use `CustomSync` when a listen address is provided.
4. Update `src/ffi.rs` to pass through the backend selection.

**Files:** `src/sdk.rs`, `src/ffi.rs`, `src/any_storage.rs`

**Acceptance criteria:**
- `BaaLSSdk::new()` accepts a config that selects sled or redb.
- FFI `baals_sdk_init` can specify backend.
- Existing FFI tests still pass.

### I.2 SDK Sync Support

**Agent instructions:**
1. Add `BaaLSSdk::add_peer()`, `BaaLSSdk::remove_peer()`, `BaaLSSdk::trigger_sync()` methods.
2. Wire through to the underlying `Runtime`'s sync layer.
3. Add corresponding FFI exports in `src/ffi.rs`.

**Files:** `src/sdk.rs`, `src/ffi.rs`

**Acceptance criteria:**
- SDK consumers can manage peers and trigger sync programmatically.
- FFI exports match the Rust SDK surface.

---

## Phase J — Spec-Gap Closure (Missing Features from Docs)

**Why:** Cross-referencing all 14 spec/compliance docs against the codebase revealed features that are documented but not in any plan phase. These are smaller tasks that fill the remaining gaps.

### J.1 Missing REST API Endpoints

**Problem:** Several REST endpoints specified in docs are not implemented in `cli/node.rs`:
- `GET /api/v1/transactions/:hash` — fetch transaction by hash
- `GET /api/v1/transactions/address/:addr` — fetch transactions by address
- `GET /api/v1/contracts/:id/state` — query contract storage state
- `GET /api/v1/proofs/:type/:key` — Merkle proof endpoints (spec uses `/api/v1/` namespace)

**Agent instructions:**
1. Add `GET /api/v1/transactions/:hash` → calls `storage.get_transaction(hash)`.
2. Add `GET /api/v1/transactions/address/:addr` → calls `storage.get_transactions_by_address(addr)` (the tx index already exists).
3. Add `GET /api/v1/contracts/:id/state` → reads contract storage via `storage.contract_storage_get_all(id)`.
4. Add proof endpoints under `/api/v1/proofs/` namespace (currently proofs are at a different path or not exposed).

**Files:** `src/cli/node.rs`

**Acceptance criteria:**
- Each new endpoint returns correct data for existing chain state.
- Invalid hash/address/id returns 404 with descriptive error.
- `cargo test api --all-features` includes tests for each new endpoint.

---

### J.2 `tx inspect` CLI Command

**Problem:** The spec mentions a `tx inspect` CLI command for decoding and displaying transaction details (payload type, gas, signatures). Not implemented.

**Agent instructions:**
1. Add `TxInspect { hash: String }` subcommand to the transaction CLI group.
2. Fetch transaction from storage by hash.
3. Display: payload type, sender, nonce, gas_limit, gas_price, chain_id, signature validity, payload-specific fields.
4. If transaction not found, return descriptive error.

**Files:** `src/cli/query.rs` or appropriate CLI module

**Acceptance criteria:**
- `tx inspect --hash <hash>` displays human-readable transaction breakdown.
- Works for all payload types: Transfer, ContractDeploy, ContractCall, Data.

---

### J.3 Wire `dev simulate-contract` to Real WASM Execution

**Problem:** `src/cli/contract.rs` has a `Simulate` command that only checks WASM magic bytes. It should perform a real dry-run using the contract engine (which already has `estimate_gas_usage()`).

**Agent instructions:**
1. In the `Simulate` handler, load the WASM bytecode and instantiate the contract engine.
2. Call `execute_contract_call()` in dry-run mode (no state mutation).
3. Return execution result: success/failure, gas used, return data, logs.
4. If the contract references other contracts (inter-contract calls), execute those too in dry-run mode.

**Files:** `src/cli/contract.rs`, `src/contracts.rs`

**Acceptance criteria:**
- `dev simulate-contract --wasm contract.wasm --method foo --args '[]'` runs real WASM execution.
- State is not mutated.
- Gas usage is reported accurately.

---

### J.4 Go SDK and Node.js SDK Integration Tests

**Problem:** `sdk/go/` has a Go HTTP client and `sdk/nodejs-native/` has napi-rs bindings, but neither has integration tests verifying they work against a running node.

**Agent instructions:**
1. In `sdk/go/`, create `baals_test.go`:
   - Start a BaaLS node (or connect to a test node).
   - Test: create wallet, submit transaction, query block, query transaction.
   - Use Go's `testing` package.
2. In `sdk/nodejs-native/`, create `__tests__/integration.test.js`:
   - Same coverage: wallet, transaction submission, block query.
   - Use Jest or Node.js built-in test runner.
3. Add CI notes: these tests require a running node and should be tagged as integration tests.

**Files:** `sdk/go/baals_test.go` (new), `sdk/nodejs-native/__tests__/integration.test.js` (new)

**Acceptance criteria:**
- Go tests pass against a running BaaLS node.
- Node.js tests pass against a running BaaLS node.
- Both test suites are documented in README with setup instructions.

---

### J.5 Generic KV Storage Methods or Document Deviation

**Problem:** Spec mentions generic `put_state`/`get_state`/`delete_state` methods for arbitrary key-value storage. The `Storage` trait has 37+ specialized methods but no generic KV interface.

**Agent instructions:**
- **Option A (implement):** Add `put_state(key: &[u8], value: &[u8])`, `get_state(key: &[u8]) -> Option<Vec<u8>>`, `delete_state(key: &[u8])` to the `Storage` trait. Implement in both sled and redb backends. These can be a simple prefixed namespace in the existing DB.
- **Option B (document deviation):** If the specialized methods sufficiently cover all use cases, document in `docs/Spec_Compliance_Notes.md` as an L3 deviation explaining that specialized methods replace the generic KV interface.
- Choose based on whether any downstream feature (SDK, WASM host functions) actually needs generic KV access.

**Files:** `src/storage.rs`, `src/redb_storage.rs`, or `docs/Spec_Compliance_Notes.md`

---

### J.6 Incremental Backup Support

**Problem:** Spec mentions incremental backup capability. Current `db backup` command does full copies. No incremental/differential backup support.

**Agent instructions:**
1. Track last backup timestamp/block height in a backup manifest file.
2. Add `db backup --incremental` flag that only exports blocks and state changes since last backup.
3. Add `db restore --incremental <path>` that applies incremental backup on top of a base.
4. Store backup manifests alongside backup data.

**Files:** `src/cli/db.rs` (or wherever db commands live), `src/storage.rs`

**Acceptance criteria:**
- Full backup followed by incremental backup → restore produces identical state.
- Incremental backup is smaller than full backup.
- `db backup --incremental` fails gracefully if no prior full backup exists.

---

### J.7 Transaction Receipt Storage

**Problem:** The codebase stores transactions but has no dedicated receipt storage. Transaction receipts (gas used, success/failure status, logs, contract address for deployments) are computed during `apply_block` but not persisted separately.

**Agent instructions:**
1. Define a `TransactionReceipt` struct in `types.rs`: `tx_hash`, `block_hash`, `block_index`, `status` (success/failure), `gas_used`, `contract_address` (for deploys), `logs: Vec<LogEntry>`.
2. In `apply_block`, create a receipt for each processed transaction and include it in the `StorageBatch`.
3. Add `put_receipt`/`get_receipt` to the `Storage` trait.
4. Add `GET /api/v1/transactions/:hash/receipt` endpoint.

**Files:** `src/types.rs`, `src/ledger.rs`, `src/storage.rs`, `src/redb_storage.rs`, `src/cli/node.rs`

**Acceptance criteria:**
- Every processed transaction has a receipt in storage.
- `GET /api/v1/transactions/:hash/receipt` returns receipt with gas_used and status.
- Deploy transactions include `contract_address` in receipt.

---

### J.8 WASM Module Caching

**Note:** This is a performance optimization, not a correctness issue. Prioritize only if contract execution latency is a concern.

**Problem:** Each contract call recompiles the WASM module from bytecode. `wasmtime` supports pre-compiled module caching that can significantly reduce repeat execution latency.

**Agent instructions:**
1. Add a module cache (LRU) to `BaaLSContractEngine`: `HashMap<CodeHash, wasmtime::Module>`.
2. On first call to a contract, compile and cache the module.
3. On subsequent calls, reuse the cached module.
4. Invalidate cache entry when contract is redeployed.
5. Make cache size configurable (`contracts.module_cache_size`, default 100).

**Files:** `src/contracts.rs`, `src/config.rs`

**Acceptance criteria:**
- Second call to same contract is measurably faster than first.
- Redeployment invalidates cache.
- Cache respects size limit.

---

## Execution Checklist

```
[ ] Phase 0: Fix Compilation (BLOCKING — nothing else can proceed)
  [ ] 0.1 Complete HostState gas metering migration in contracts.rs

[ ] Phase 0A: Architectural Prerequisites (BLOCKING — fix before feature work)
  [ ] 0A.1 Fix block hash→sign pipeline (consensus.rs, types.rs)
  [ ] 0A.2 Fix inter-contract call results bug (contracts.rs)
  [ ] 0A.3 Add typed fields to Block struct (types.rs, consensus.rs, ledger.rs)
  [ ] 0A.4 Document spec deviations (docs/Spec_Compliance_Notes.md)

[ ] Phase A: Multi-Validator PoA (depends on 0A.1, 0A.3)
  [ ] A.1 Quorum threshold validation
  [ ] A.2 Round-robin signer scheduling
  [ ] A.3 Validator set change transactions

[ ] Phase B: Rollback Logs / Snapshots
  [ ] B.1 Write-ahead rollback log
  [ ] B.2 Snapshot-based recovery

[ ] Phase C: P2P CLI Commands
  [ ] C.1 Wire p2p peers
  [ ] C.2 Wire add-peer / remove-peer / sync-now

[ ] Phase D: Contract / Admin CLI (D.1 depends on 0A.2 for full contract engine health)
  [ ] D.1 Wire contract estimate-gas CLI to engine
  [ ] D.2 contract abi
  [ ] D.3 admin rotate-consensus-key
  [ ] D.4 admin tls-generate

[ ] Phase E: WebSocket API (separate port strategy — no tiny_http rewrite)
  [ ] E.1 WebSocket server on dedicated port (requires tokio + tokio-tungstenite)
  [ ] E.2 Update docs

[ ] Phase F: Spec Alignment
  [ ] F.1 Block hash versioning for production chains (skip if no legacy data)
  [ ] F.2 Document metadata deviation

[ ] Phase G: Testing & Hardening (depends on A + B)
  [ ] G.1 Multi-validator integration tests (depends on A)
  [ ] G.2 Reorg stress tests (depends on B)
  [ ] G.3 Crash recovery tests (depends on B)
  [ ] G.4 Byzantine fault tests (depends on A + 0A.2)

[ ] Phase H: Documentation
  [ ] H.1 Update all spec docs
  [ ] H.2 Add gap closure progress tracker

[ ] Phase I: SDK / FFI Hardening
  [ ] I.1 Make BaaLSSdk generic over storage backend
  [ ] I.2 SDK sync support

[ ] Phase J: Spec-Gap Closure
  [ ] J.1 Missing REST API endpoints (tx-by-hash, tx-by-address, contract state, proofs)
  [ ] J.2 tx inspect CLI command
  [ ] J.3 Wire dev simulate-contract to real WASM execution
  [ ] J.4 Go SDK and Node.js SDK integration tests
  [ ] J.5 Generic KV storage methods or document deviation
  [ ] J.6 Incremental backup support
  [ ] J.7 Transaction receipt storage
  [ ] J.8 WASM module caching (performance optimization — lower priority)
```

---

## Quality Gates (Run After Every Phase)

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

All three must pass before moving to the next phase.

---

## Definition of Done

1. All code compiles with zero errors and zero warnings under `clippy -D warnings`.
2. All tests pass (`cargo test --all-features`).
3. No `warn!`-and-continue paths remain in `apply_block`.
4. Every documented feature has a corresponding test.
5. Compliance docs match actual code + test results.
6. No placeholder/synthetic success paths in CLI commands.
7. SDK (`sdk.rs`) and FFI (`ffi.rs`) expose all runtime capabilities.
8. No consensus-breaking changes without a versioned upgrade path (fork height or version flag).
9. Block hash covers signer identity — no unsigned data in consensus-critical fields.
10. Inter-contract calls are fully functional — call results readable by calling contract.
11. All spec deviations are documented with rationale in `docs/Spec_Compliance_Notes.md`.
12. Transaction receipts are persisted and queryable via REST API.
13. Go and Node.js SDKs have passing integration tests.
