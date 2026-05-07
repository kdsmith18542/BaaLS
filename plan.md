# BaaLS Development Plan — Final Execution (All Gaps)

**ALL CODE MUST BE PRODUCTION GRADE. NO STUBS. NO DEFERRALS. NO SKIPS.**

## Status Overview

| Metric | Value |
|---|---|
| Phases 1-26 | Complete |
| Spec compliance | 135/157 requirements (86%) |
| Tests | 49 pass (12 lib + 37 integration) |
| Clippy | 0 errors, 0 warnings |
| Rustfmt | 0 diffs |
| Build | 0 warnings |

---

## Remaining Work: 13 Gaps to Zero

### Track A: Storage Compaction (GAP-1, GAP-2)

**SledStorage compaction** — Current `compact()` only flushes trees. Real compaction must:
1. Scan each tree for stale/overwritten keys.
2. Build a new clean tree with only live entries.
3. Atomically swap via batch write.
4. Update WAL to allow crash recovery during compaction.

**RedbStorage compaction** — Current `compact()` is `Ok(())`. Implement by:
1. Opening a new database file.
2. Copying all live entries from old to new.
3. Closing old DB and atomically replacing the file.
4. Re-opening the new database.

**Files**: `src/storage.rs`, `src/redb_storage.rs`, `src/any_storage.rs`

---

### Track B: Multi-Validator PoA Consensus (GAP-3)

**Goal**: Support multiple authorized signers, not just one.

1. Add `authorized_signers: Vec<PublicKey>` to `PoAConsensus`.
2. Change `validate_block` to accept any signer in the authorized set.
3. Add `add_authorized_signer` / `remove_authorized_signer` methods.
4. Support signer rotation via metadata.
5. Ensure `generate_block` still uses the local signing key.

**Files**: `src/consensus.rs`, `src/types.rs`

---

### Track C: Peer Discovery via mDNS (GAP-4)

**Goal**: Auto-discover peers on LAN without manual `--peer` flags.

1. Add optional `mdns` crate dependency (behind feature flag).
2. Implement `MdnsDiscovery` struct in `src/sync.rs`.
3. Periodically broadcast node ID + listen addr via mDNS.
4. Listen for other BaaLS nodes and auto-add them to `known_peers`.
5. Add `--mdns` CLI flag to enable.

**Files**: `src/sync.rs`, `src/main.rs`, `Cargo.toml`

---

### Track D: Contract Engine Validation (GAP-5, GAP-6)

**ContractDeployerAddress validation (GAP-20)** — Previously skipped. Implement:
1. Add `ContractDeployerAddress` type that wraps a `PublicKey`.
2. In `deploy_contract`, verify the deployer matches the transaction sender.
3. Store deployer address with contract code.
4. Add `get_contract_deployer()` to `Storage` trait.

**Contract ABI/export validation**:
1. In `validate_wasm_module`, check that the requested method exists as an export.
2. Add `ContractAbi` struct with method signatures.
3. Validate argument counts match ABI on `call_contract`.
4. Reject deployment if no valid exports are found.

**Files**: `src/contracts.rs`, `src/types.rs`, `src/storage.rs`, `src/redb_storage.rs`, `src/any_storage.rs`

---

### Track E: Wire reorganize_chain() into Sync Flow (GAP-7)

**Goal**: Actually use the existing `reorganize_chain()` method.

1. In `sync_with_peer`, when peer is ahead by >1 block, request full block range.
2. Call `reorganize_chain()` with received fork blocks instead of applying one-by-one.
3. Handle reorg failures gracefully (keep local chain).
4. Add integration test for chain reorganization.

**Files**: `src/sync.rs`, `src/runtime.rs`

---

### Track F: HTTP API & CLI Improvements (GAP-8, GAP-9, GAP-10)

**Sparse Merkle proof exposure**:
1. Add `/proof/account/{address}` endpoint to health server.
2. Add `/proof/contract/{contract_id}/storage/{key}` endpoint.
3. Return JSON with `{ root, proof: [...], value }`.

**simulate-contract CLI improvements**:
1. Add gas estimate to output.
2. Show state changes (storage writes/reads).
3. Show events emitted.
4. Show execution time.

**Log rotation**:
1. Add `log_max_size_mb` and `log_max_files` to `Config`.
2. In `setup_logging`, use `tracing_appender::rolling` or custom rotation.
3. Rotate when file exceeds max size, keep N backups.

**Files**: `src/main.rs`, `src/config.rs`, `src/runtime.rs`, `src/ledger.rs`

---

## Verification Gates (Per Track + Final)

After each track, run:
```
cargo build --release
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
cargo test --lib
cargo test --test integration
```

All must pass with 0 errors, 0 warnings, 0 diffs before merging track.

---

## Execution Order

1. **Parallel Batch 1**: Track A (Storage) + Track B (Consensus) + Track D (Contracts)
2. **Parallel Batch 2**: Track C (mDNS) + Track E (Sync Reorg)
3. **Batch 3**: Track F (HTTP/CLI)
4. **Final Integration**: Run all tests, fix conflicts, verify 157/157.

---

## Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| Multi-validator breaks single-node tests | High | Keep single-signer as default; multi-signer opt-in via config. |
| mDNS adds new dependency | Medium | Feature-gated; disabled by default. |
| Storage compaction corrupts data | High | WAL + atomic swap; test crash recovery. |
| ABI validation breaks existing test WASM | Low | Update test fixtures to include proper exports. |

---

## Dependency Additions

| Crate | Track | Purpose |
|---|---|---|
| `mdns` (optional) | C | LAN peer discovery |
| `tracing-appender` | F | Log rotation |

---

## Target: 157/157 Requirements, 0 Deferrals, 0 Skips
