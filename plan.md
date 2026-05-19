# BaaLS Production Plan — Honest 100% Roadmap

**Last updated:** 2026-05-09  
**Current status:** Testnet-grade only. Financial mainnet requires all Phase 1–3 items.

---

## Reality check

The previous plan claimed "PHASES A–E COMPLETE." That was accurate for the
*feature checklist*, not for production correctness. The code review below
found real bugs that cause fund loss or protocol divergence today. They must
be fixed before any value-bearing use.

---

## Code Quality Review — Issues Found

### CRITICAL (fund-loss bugs, fix immediately)

#### CQ-1  Contract call value not rolled back on failure
**File:** `src/ledger.rs:347–478`  
The value transfer to the contract (lines 357–384) is pushed to the storage
batch **before** the WASM call is executed (line 389). If the call returns
`Err`, the code does `warn!` and continues. The batch is then committed
atomically with the debit already in it. Outcome: funds are silently
transferred into the contract and locked there permanently.

**Fix:** Collect value-transfer ops in a separate `call_ops` vec. On `Err`,
discard `call_ops`; on `Ok`, append them to the main batch before commit.

```rust
// before the call:
let mut call_ops: Vec<StorageOperation> = Vec::new();
// use call_ops.push(...) for the value-transfer accounts
// after call_contract:
match result {
    Ok(_) => batch.ops.extend(call_ops),
    Err(e) => { warn!("..."); /* call_ops dropped, no debit */ }
}
```

#### CQ-2  Gas fee overcharge when operation exceeds gas limit
**File:** `src/ledger.rs:246, 343, 484, 492–495`  
When an operation exceeds the gas limit (e.g. deploy uses 121 000 gas but
limit is 50 000) the operation is skipped but `gas_used` still reflects
the excess amount. The fee at line 492 is `gas_price * gas_used`, which
exceeds `gas_price * gas_limit`. Users are overcharged for operations that
never executed.

**Fix:** Cap `gas_used` to `gas_limit` before fee computation.

```rust
let billable_gas = gas_used.min(tx.gas_limit);
let total_fee = tx.gas_price.checked_mul(billable_gas)…;
```

#### CQ-3  No balance check before mempool acceptance
**File:** `src/runtime.rs:525–660`  
`submit_transaction` validates signature, timestamp, gas bounds, payload
format, and nonce — but **never checks that the sender has enough balance**
to cover `amount + gas_price * gas_limit`. A 0-balance account can flood the
mempool with transfer transactions that will fail silently in `apply_block`.
This wastes block space and causes empty nonce increments.

**Fix:** Add balance check in step 5 of `submit_transaction`:

```rust
let max_cost = transaction.gas_price
    .saturating_mul(transaction.gas_limit)
    .saturating_add(transfer_amount); // 0 for non-transfers
if sender_account.balance() < max_cost {
    return Err(RuntimeError::InvalidTransaction("Insufficient balance".into()));
}
```

#### CQ-4  No chain_id in transaction hash — cross-chain replay
**File:** `src/types.rs:730–757`  
`Transaction::calculate_hash` does not include a chain/network identifier.
A transaction signed for a testnet is byte-identical on mainnet (same keys,
same nonce, same payload). Replaying it on mainnet drains the sender's
mainnet balance.

**Fix:** Add `chain_id: u64` to `Transaction` (and `Block`). Include it in
`calculate_hash`. Validate in `validate_block` that `tx.chain_id == node.chain_id`.

#### CQ-5  Fork resolution is forward-only — true reorg not implemented
**File:** `src/runtime.rs:980–1043`  
`reorganize_chain` only applies blocks if `fork_height > local_height`. It
cannot switch to a fork that diverges at a lower block. When two nodes
build different chains from the same common ancestor, neither can adopt the
other's chain. Network fragments permanently.

**Fix:** Implement a proper reorg:
1. Walk back to common ancestor (binary search on hashes or linear scan).
2. Unapply (rollback) local blocks from tip to ancestor using a snapshot
   or WAL.
3. Apply fork blocks forward to new tip.
4. Only adopt fork if it is longer than local.

#### CQ-6  gas_price defaults to 0 everywhere in CLI
**File:** `src/main.rs:2313, 2434`  
All CLI-created transactions hardcode `gas_price: 0`. The fee calculation
`gas_price * gas_used = 0` means the fee market never runs; blocks are
filled with 0-cost transactions; the block producer earns nothing; spam
resistance is zero.

**Fix:** Make `gas_price` a required or defaulted CLI flag with a sensible
minimum (e.g. 1 unit). Enforce a minimum gas price in `submit_transaction`.

---

### HIGH (protocol correctness)

#### CQ-7  Authorized signers list not persisted — lost on restart
**File:** `src/consensus.rs:47, 59`  
`authorized_signers: Vec<PublicKey>` is in-memory only. Any additional
signers added at runtime via `add_authorized_signer` are wiped when the node
restarts. After restart, only the primary key in config can produce blocks.

**Fix:** Persist the authorized signers list to storage (new `ConsensusState`
table). Load at startup, update on every add/remove.

#### CQ-8  Block timestamps not monotonically enforced
**File:** `src/consensus.rs:82–139` (validates future only), `src/ledger.rs:115–144` (no timestamp check)  
The consensus validator checks that a block's timestamp is not more than 10 s
in the future but does **not** require `block.timestamp >= prev_block.timestamp`.
A bad signer could produce blocks with decreasing timestamps, breaking any
time-dependent contract logic and audit trails.

**Fix:** In `validate_block`, load the parent block and assert:
```rust
if block.timestamp < prev_block.timestamp {
    return Err(ConsensusError::InvalidTimestamp);
}
```

#### CQ-9  total_supply never updated
**File:** `src/ledger.rs:107, 569`  
Genesis initialises `total_supply: 0`. Every subsequent `apply_block` carries
it forward unchanged. No mint or fee-collection path updates it. A supply
tracker showing 0 forever undermines any economic analysis or audit.

**Fix:** Track net balance changes in `apply_block` (newly minted tokens
during genesis allocations, fees burned, etc.) and update `total_supply`
accordingly.

#### CQ-10  P2P accepts any authenticated peer — no authorization list
**File:** `src/sync.rs`  
The challenge-response handshake proves a peer owns its key but does not
check against an authorized list. Any node in the world can connect to your
node if it can reach the port. On a private financial chain this is wrong.

**Fix:** Add `authorized_peer_keys: HashSet<PublicKey>` to `SyncLayer`. If
non-empty, reject handshakes from keys not in the set. Expose a config
option `allowed_peers` in `config.toml`.

#### CQ-11  Silent failure on insufficient-balance transfer included in block
**File:** `src/ledger.rs:207–208`  
Failed transfers log `warn!` and continue. The transaction is included in the
block, nonce increments, no fund movement occurs. Users see a "confirmed"
transaction that did nothing. This is hard to reconcile for wallets and
exchanges.

**Fix:** Either:  
(a) Return an error from `apply_block` to reject the block (strict mode), or  
(b) Record a `TransactionStatus::Failed(reason)` in the transaction index
    and expose it via `query tx`.  
Option (b) matches Ethereum semantics. Option (a) is simpler for PoA.

#### CQ-12  Transaction duplicate check only in mempool, not in ledger
**File:** `src/ledger.rs:115–144`  
`validate_block` verifies signatures and block-level hash/chain integrity but
does not verify that the same transaction hash does not already appear in
storage. After a reorg, a previously-included transaction can be re-included
in the fork chain.

**Fix:** In `validate_block`, check `storage.get_transaction(tx.hash)` for
each tx. If already stored (from a committed block), reject.

---

### MEDIUM (code quality / debt)

#### CQ-13  main.rs is 3 846 lines — unmaintainable
Split into:
- `src/cli/wallet.rs`
- `src/cli/tx.rs`
- `src/cli/query.rs`
- `src/cli/node.rs`
- `src/cli/admin.rs`
- `src/cli/p2p.rs`
- `src/cli/contract.rs`
- `src/cli/db.rs`
- `src/cli/dev.rs`

#### CQ-14  Two gas accounting systems that can diverge
`contracts.rs` runs wasmtime's native fuel counter **and** a manual
`charge_gas` counter. If the two disagree (e.g. wasmtime fuel is consumed
before `charge_gas` is called), the reported `gas_used` from the contract
won't match the wasmtime fuel consumed. Use **one** system only — prefer
wasmtime fuel as it is enforced by the VM.

#### CQ-15  Warn-and-continue pattern hides state divergence
Eight locations in `ledger.rs` log `warn!` and silently continue rather than
returning errors or recording failure status. This means two nodes running
the same block may produce different state roots if one node is in a slightly
different state when the warn is triggered. All `warn!`-and-continue in
`apply_block` should either fail-fast (return `Err`) or record a
deterministic failed-tx record.

#### CQ-16  No minimum gas price protocol parameter
The chain has no on-chain minimum gas price. Full nodes can set their own
floor but block producers accept 0-fee transactions, making DoS trivial.
Add `min_gas_price: u64` to `Config` and enforce in both `submit_transaction`
and `apply_block`.

#### CQ-17  Block hash does not cover block-level gas metrics
The block hash (types.rs:706–726) covers index, timestamp, prev_hash, nonce,
and tx Merkle root. It does not cover `total_gas_used`, `miner/signer`, or
`state_root`. Light clients cannot verify the state root from the block hash
alone. Add `state_root` to the block header and include it in the hash.

---

## Phase 1 — Bug Fixes (must complete before any value-bearing use)

Estimated effort: 1–2 weeks

| # | Item | File(s) | Est |
|---|------|---------|-----|
| 1.1 | Fix CQ-1: rollback contract value on call failure | ledger.rs | 1d |
| 1.2 | Fix CQ-2: cap gas_used at gas_limit for fee computation | ledger.rs | 2h |
| 1.3 | Fix CQ-3: add balance check to submit_transaction | runtime.rs | 2h |
| 1.4 | Fix CQ-4: add chain_id to Transaction + Block hash | types.rs, config.rs | 1d |
| 1.5 | Fix CQ-6: remove gas_price=0 default; enforce min gas price | main.rs, runtime.rs | 4h |
| 1.6 | Fix CQ-7: persist authorized signers to storage | consensus.rs, storage.rs | 4h |
| 1.7 | Fix CQ-8: enforce monotonic block timestamps | consensus.rs | 2h |
| 1.8 | Fix CQ-11: record failed tx status in index | ledger.rs, storage.rs | 4h |
| 1.9 | Fix CQ-12: dedup transactions against committed storage | ledger.rs | 2h |
| 1.10 | Fix CQ-16: add min_gas_price to Config and enforce it | config.rs, runtime.rs | 2h |
| 1.11 | Add state_root to block header + hash (CQ-17) | types.rs, ledger.rs | 1d |
| 1.12 | Add regression tests for all 11 bugs above | tests/ | 2d |

---

## Phase 2 — Protocol Completeness

Estimated effort: 3–4 weeks

### 2.1 True chain reorganization
Implement a full reorg with rollback to common ancestor (CQ-5).  
This requires either:
- **Snapshot / copy-on-write storage** — snapshot before applying each block, discard on reorg  
- **Write-ahead log** — record inverse ops for each block, replay on rollback  

Recommended: WAL approach. Add `RollbackLog` to storage layer.

### 2.2 Multi-validator PoA with quorum
Current: single signer, SPOF.  
Target: configurable `M-of-N` quorum (e.g. 3-of-5).

- Block must carry `M` signatures (one per signer)
- `validate_block` counts valid signatures; fails if count < M
- All N signers' public keys stored on-chain in a `ValidatorSet` record
- Validator set changes require M signatures and a block-height timelock

### 2.3 Formal finality
Add `finality_depth: u64` to config (default: 12).  
Expose `GET /tx/{hash}/finality` returning:
- `{ "final": false, "confirmations": 3, "required": 12 }`
- `{ "final": true, "confirmations": 12 }`

Block producer must not include transactions conflicting with finalized blocks.

### 2.4 Authorized peer list
Implement `allowed_peers` in config (CQ-10).  
When non-empty, the sync layer rejects handshakes from unknown keys.

### 2.5 Total supply tracking
Implement `total_supply` accounting (CQ-9):
- Genesis sets initial supply per initial account allocations
- Fee collection increments a `fee_pool` account
- Explicit `Mint` and `Burn` payloads (admin-only, gated by consensus) adjust supply
- `query supply` CLI command

### 2.6 Unified gas accounting
Remove the manual `charge_gas` counter and rely entirely on wasmtime fuel
(CQ-14). Expose `gas_used = gas_limit - store.get_fuel()`.

### 2.7 Transaction status in query layer
Index `TransactionStatus { Success, Failed(String), Pending }` alongside
each tx record (CQ-11). Expose via `query tx <hash>` and HTTP `/tx/{hash}`.

### 2.8 HTTPS for HTTP API
Replace `tiny_http` (plain HTTP) with a TLS-enabled server (e.g. `rustls`
+ `hyper`). Require HTTPS for all mutating endpoints. Keep plain HTTP only
for `/health` on loopback.

### 2.9 Short-lived JWT admin tokens
Replace the static `BAALS_ADMIN_TOKEN` env var with HMAC-SHA256 JWTs:
- `POST /auth/token` (loopback-only, requires node signing key)
- Tokens expire in 15 minutes
- Rotating key invalidates all issued tokens

---

## Phase 3 — Production Hardening

Estimated effort: 4–6 weeks

### 3.1 Formal SMT audit
The `SparseMerkleTree` in `types.rs` is hand-rolled. Before any light-client
or cross-chain proof use, commission an independent review of:
- Proof generation correctness
- Proof verification (no false positives)
- Absence proofs
- Key collision resistance

### 3.2 Full fork-choice rule + reorg depth limit
- Implement longest-chain-first fork choice
- Add `max_reorg_depth: u64` (default: 50) — reject forks older than this
- Log all reorg events with before/after state roots

### 3.3 Byzantine-fault testing
Add tests for:
- 3-node network with one Byzantine peer sending invalid blocks
- Validator producing two conflicting blocks at same height (equivocation)
- Peer that disconnects mid-block-send
- Eclipse attack simulation (all peers replaced by one attacker)

### 3.4 Storage integrity checksums
Add SHA256 checksum to every stored block record.  
`db verify` recomputes and compares checksums, reports corrupted records.

### 3.5 Crash-recovery stress tests
- Kill node mid-`apply_block` (inject panic), verify restart produces correct state
- Kill node during backup, verify partial backup is not restoreable
- Kill node during DB migration, verify migration is idempotent

### 3.6 WASM sandbox audit
Commission or run automated analysis of the wasmtime host functions for:
- All host functions that can mutate state — verify they charge gas first
- `storage_write` called after gas exhaustion — should be blocked
- Memory exports — verify no WASM module can access host memory

### 3.7 Refactor main.rs (CQ-13)
Split 3 846-line `main.rs` into CLI submodules. Each group (wallet, tx,
query, etc.) gets its own file. `main.rs` becomes ~100 lines of top-level
dispatch.

### 3.8 Immutable audit log
Every admin action (key rotation, signer change, backup, restore) writes a
signed, append-only record to `admin_log.jsonl`. Log entries include:
- timestamp, operator key, action, affected keys/params, signature

### 3.9 Key rotation ceremony
Define a formal `rotate-consensus-key` workflow:
1. New key generated on airgap machine
2. Rotation transaction signed by quorum of current validators
3. Rotation included in block N with timelock (effective at block N+1000)
4. Nodes that haven't updated reject blocks after N+1000 until updated

### 3.10 Metrics and alerting
Expose Prometheus-compatible metrics at `/metrics` (loopback-only):
- `chain_height`, `mempool_size`, `block_time_ms`, `peer_count`
- `failed_txs_per_block`, `gas_used_per_block`
- Alert thresholds: peer_count < 2, block_time > 30s, failed_txs > 10%

---

## Phase 4 — Audit, Certification, Launch

Estimated effort: 4–8 weeks depending on audit scope

### 4.1 External security audit
Scope:
- Consensus + fork resolution
- Cryptographic primitives (key derivation, signature verification, SMT)
- WASM sandbox isolation
- Network protocol (P2P handshake, message handling)
- HTTP API auth

### 4.2 Fuzz campaign (minimum 24-hour runs)
Fuzz all 7 targets:
```
cargo fuzz run fuzz_contract_execution      -- -max_total_time=86400
cargo fuzz run fuzz_ledger_state_transition -- -max_total_time=86400
cargo fuzz run fuzz_merkle_proof            -- -max_total_time=86400
cargo fuzz run fuzz_sync_messages           -- -max_total_time=86400
cargo fuzz run tx_decode                    -- -max_total_time=86400
cargo fuzz run block_decode                 -- -max_total_time=86400
cargo fuzz run ffi_json_decode              -- -max_total_time=86400
```

### 4.3 Load test at target TPS
Target: 1 000 TPS sustained for 1 hour on 3-node network.  
Metrics: p50/p99 block time, mempool queue depth, DB size growth rate,
CPU/RAM per node.

### 4.4 Testnet operations
Run a public testnet for ≥ 30 days with:
- At least 3 independent validator nodes
- Public faucet + explorer
- Monitored uptime, reorg events, fork incidents

### 4.5 Release candidate checklist
```
[x] All Phase 1 bugs fixed and regression-tested
[ ] Phase 2 protocol items complete and tested
[ ] Phase 3 hardening items complete
[ ] External audit report reviewed, critical/high findings resolved
[ ] Fuzz campaign completed, no crashes or assertion failures
[ ] Load test passed at target TPS
[ ] Testnet run ≥ 30 days with 0 consensus failures
[x] CHANGELOG.md updated
[x] Reproducible build: cargo build --release --locked produces same binary
[x] Binary checksums published in RELEASES.md
[x] SECURITY.md updated with bug-bounty scope
```

---

## Current Test Coverage vs Required

| Area | Current | Required for Mainnet |
|------|---------|---------------------|
| Unit tests | 18 ✅ | 18+ ✅ |
| Integration tests | 44 ✅ | 50+ (after Phase 1 fixes) |
| Security tests | 3 ✅ | 20+ (Byzantine, crash, SMT) |
| CLI lifecycle | 1 ✅ | full smoke suite |
| CQ bug regression tests | **11 ✅** | 11 (Phase 1 bug set: CQ-1,2,3,4,6,7,8,11,12,16,17) |
| True reorg tests | **0 ❌** | 5+ |
| Multi-validator consensus | **0 ❌** | 10+ |
| Fuzz (24h runs) | **0 ❌** | 7 targets |
| Load test (1h 1k TPS) | **0 ❌** | 1 documented run |
| Crash recovery | **0 ❌** | 5+ |

---

## What "100% ready" actually means

A financial blockchain is ready for production when:

1. **Correctness:** Every committed block produces the same state root on every
   node. There are no code paths that silently diverge (warn-and-continue).

2. **Safety:** Funds cannot be lost due to a protocol bug. Contract value
   rollbacks work. Gas fees are correct. Replay attacks are impossible.

3. **Liveness:** A single node failure (including the primary signer) does not
   halt the chain permanently. Multi-validator quorum + key rotation ceremony.

4. **Finality:** Users can know with certainty when a transaction is irreversible.
   Defined finality depth. No unbounded reorg.

5. **Auditability:** Every state change is traceable. Transaction status is
   queryable. Failed txs are recorded. Admin actions are logged immutably.

6. **Security:** External audit of cryptographic + network code. Fuzz campaign
   completed. No known high/critical open findings.

---

## Summary of what exists today

| Property | Status | Blocks mainnet? |
|----------|--------|-----------------|
| Basic transfer + nonce ordering | ✅ works | No |
| Ed25519 signature verification | ✅ correct | No |
| WASM contract execution (wasmtime) | ✅ with fuel | No |
| P2P sync with mutual auth | ✅ handshake works | No |
| Keystore (Argon2id + AES-GCM) | ✅ solid | No |
| CLI (80+ commands) | ✅ implemented | No |
| Contract value rollback on failure | **❌ bug CQ-1** | **Yes** |
| Gas fee correctness | **❌ bug CQ-2** | **Yes** |
| Balance check before mempool | **❌ bug CQ-3** | **Yes** |
| Cross-chain replay protection | **❌ no chain_id** | **Yes** |
| True fork/reorg resolution | **❌ forward-only** | **Yes** |
| Multi-validator quorum | **❌ single signer** | **Yes** |
| Transaction finality | **❌ no depth/API** | **Yes** |
| Block state root in header | **❌ not in hash** | **Yes** |
| Authorized signers persist | **❌ in-memory** | **Yes** |
| Monotonic block timestamps | **❌ not enforced** | Partial |
| Minimum gas price | **❌ always 0** | **Yes** |

---

## 2026-05-19 Completeness Remediation Plan (Spec + Current Status)

Scope: resolve all findings from the latest completeness review comparing `docs/` specs, declared compliance status, and current behavior.

### Priority 0: Restore truthful quality/status reporting

#### T0.1 Fix active quality gate failures
Problem: `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings` fail on current tree.
Tasks:
1. Fix clippy violations in `src/cli/contract.rs` and `src/ledger.rs`.
2. Run and pass:
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --all-features`
Acceptance criteria:
- All three commands pass on head.
- CI mirrors the same pass state.

#### T0.2 Reconcile compliance claims with evidence
Problem: compliance docs claim pass status that can drift from reality.
Tasks:
1. Update `docs/COMPLIANCE.md` with a timestamped status section from fresh command output.
2. Update summary metrics in `docs/Spec_Compliance_Notes.md` to match actual results.
3. Add a reproducibility note listing exact commands used.
Acceptance criteria:
- No stale gate/test counts.
- No contradictions across compliance docs.

### Priority 1: Align API contract across spec, CLI, and server

#### T1.1 Define canonical endpoint namespace
Problem: spec/CLI reference `/api/v1/*`; server currently exposes different paths.
Tasks:
1. Choose canonical namespace:
   - recommended: adopt `/api/v1/*` in server
   - or update docs/CLI to existing routes (if intentionally final)
2. Publish route mapping table (legacy -> canonical) in `docs/OPERATING.md`.
3. Keep compatibility aliases for one release if paths change.
Acceptance criteria:
- Docs, CLI guidance, and server routes all agree.

#### T1.2 Add endpoint contract tests
Tasks:
1. Add tests for published paths covering submit tx, account query, block query, contract read call, and health.
2. Fail tests when documented path and runtime path diverge.
Acceptance criteria:
- Published API works exactly as documented.

### Priority 2: Replace placeholder command behavior with real outcomes

#### T2.1 P2P command group
Problem: `p2p` commands currently rely on local file placeholders and synthetic responses.
Tasks:
1. Wire `peers`, `add-peer`, `remove-peer`, `sync-now` to runtime/sync state.
2. Remove misleading synthetic success paths.
3. Return explicit `not implemented` (non-zero) for any remaining placeholders.
Acceptance criteria:
- P2P command output reflects live runtime behavior.

#### T2.2 Contract/admin/api command groups
Problem: several commands are informational stubs but can look complete.
Tasks:
1. Implement real behavior for `contract abi`, `contract estimate-gas`, `admin rotate-consensus-key`, `admin tls-generate`, and `api` request flows; or
2. Mark as explicitly experimental/not implemented with truthful exit semantics.
Acceptance criteria:
- No command reports successful execution for non-executed operations.

### Priority 3: SDK/platform completeness reconciliation

#### T3.1 Supported SDK matrix
Problem: docs imply broader SDK/API availability than current repo implementation.
Tasks:
1. Define support tiers (`GA`, `Beta`, `Planned`) in docs.
2. Mark Python SDK and WebSocket streaming according to actual state.
3. Align `README.md` and `docs/BaaLS_CLI_SDK_Wiring_Overview.md` to same matrix.
Acceptance criteria:
- No availability claim without shipped implementation.

#### T3.2 Node SDK packaging cleanup
Problem: `sdk/nodejs` and `sdk/nodejs-native` both claim `@baals/sdk` and native package metadata references entry files not present in source tree.
Tasks:
1. Resolve package naming/versioning strategy to avoid collision.
2. Commit required entrypoints or enforce build-generation + CI validation.
3. Document one clear installation path per variant.
Acceptance criteria:
- Deterministic install/publish flow without package confusion.

### Priority 4: Resolve consensus/compliance narrative contradictions

#### T4.1 Consensus status consistency
Problem: docs conflict on whether multi-validator PoA is deferred or resolved.
Tasks:
1. Decide canonical status from code + tests.
2. Update `docs/Spec_Compliance_Notes.md` and `docs/COMPLIANCE.md` accordingly.
3. Link status claims to concrete test coverage.
Acceptance criteria:
- No contradictory consensus status statements remain.

### Execution order
1. T0.1 -> T0.2
2. T1.1 -> T1.2
3. T2.1 + T2.2
4. T3.1 -> T3.2
5. T4.1

### Definition of done for this remediation block
1. All review findings fixed or truthfully marked deferred.
2. Published commands/endpoints execute as documented.
3. Compliance docs are evidence-backed and reproducible.

### Execution status (2026-05-19)
- [x] T0.1 quality gates fixed (`fmt`/`clippy`/`test` passing)
- [x] T0.2 compliance docs reconciled with fresh evidence
- [x] T1.1 canonical `/api/v1/*` API namespace wired with legacy aliases
- [x] T1.2 endpoint contract checks added to CLI lifecycle test
- [x] T2.1 `p2p` placeholder success paths removed (explicit not implemented)
- [x] T2.2 `contract`/`admin` placeholder success paths removed; `api` commands execute live HTTP requests
- [x] T3.1 SDK/support matrix updated in docs
- [x] T3.2 Node package collision resolved and native entry files added
- [x] T4.1 consensus/compliance narrative contradictions reconciled
- [x] Added CQ-1 regression coverage to `tests/cq_regression.rs` (suite now 11/11 for Phase 1 bug set)
- [x] Phase 2.3/2.7 progress: transaction status + finality surfaced via runtime, `query tx`, and `/api/v1/transactions/{hash}[ /finality ]`; batch tx->block index mapping repaired for sled/redb
