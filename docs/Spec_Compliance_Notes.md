# Spec Compliance Notes

Last updated: 2026-05-20

This file records intentional, current design deviations from the specification documents under `docs/`.

## Current Evidence

- `cargo fmt --all -- --check` passes
- `cargo clippy --all-targets --all-features -- -D warnings` passes
- `cargo test --all-features` passes (77 passed, 0 failed, 1 ignored)

## Type / Data Model Deviations

### L1: Crate named `baals` instead of `libchain`
- Spec reference: `BaaLS_Core_Engine_Runtime.md`
- Rationale: `libchain` was a placeholder name; `baals` is the shipped crate identity.

### L2: Hash fields use `[u8; 32]` instead of `String`
- Spec reference: `BaaLS_Core_Engine_Runtime.md`
- Rationale: fixed-size bytes are safer and deterministic; hex is rendered at boundaries.

### L3: Metadata uses `BTreeMap<String, String>`
- Spec reference: `BaaLS_Core_Engine_Runtime.md`
- Rationale: deterministic ordering and lower complexity versus nested JSON values.

### L4: Transaction recipient is `Address` enum
- Spec reference: `BaaLS_Core_Engine_Runtime.md`
- Rationale: explicit modeling of wallet vs contract recipients.

### L5: Transaction has `gas_price` field despite no-fee mode
- Spec reference: `BaaLS_Transactions.md` — "no fees in local mode"
- Current implementation: `gas_price` field exists and is accepted but set to 0 in PoA mode.
- Rationale: reserves field for future fee-market pluggability; zero-cost in current implementation.

### L6: Failed transactions still increment nonce and deduct gas
- Spec reference: `BaaLS_Ledger_State_Transitions.md` — implies atomic failure mode
- Current implementation: nonce increments and gas deducts even when transaction fails (e.g., insufficient balance).
- Rationale: intentional spam-prevention measure; failed transactions have real cost to compute.

### L7: `Account::Contract` has no balance field
- Spec reference: `BaaLS_Core_Engine_Runtime.md` — mentions contract balances
- Current implementation: `Account::Contract` struct lacks balance field; contracts have no native balance in PoA mode.
- Rationale: contract asset holding deferred to fungible-token standard implementation phase; will be added when token system is designed.

## Runtime / API Deviations

### L8: `Runtime::start()` is lifecycle-only by default
- Auto block production is opt-in via runtime config.
- Rationale: embedded/test use-cases need explicit control.

### L9: Generic KV Storage Methods (spec → specialized API)
- Spec reference: `BaaLS_Core_Engine_Runtime.md` — generic `put_state`/`get_state`/`delete_state` methods.
- Current implementation: `Storage` trait has 37+ specialized methods (e.g., `put_block`, `put_transaction`, `get_account`, `contract_storage_write`) instead of a generic KV interface.
- Rationale: specialized methods provide type safety, domain-specific validation, and optimized backend access patterns; `contract_storage_write/read/remove` fill the generic-KV role for WASM contract state only.
- Severity: L3 (Intentional deviation — design choice)

## Historical Resolutions (Retained)

The following major historical gaps are resolved in the current tree:

- Block hash uses transaction Merkle root
- Contract deploy supports `init_payload`
- WASI block context parameters are wired
- `baals_call_contract` includes value parameter
- Config block time is wired into consensus/runtime
- Block overflow transaction rejection is enforced
- Auto-block threshold bypass behavior removed
- Wallet delete CLI command exists
- Config key coverage for storage/network fields expanded
- Multi-validator PoA (quorum, round-robin, authorized signer rotation)
- WebSocket streaming endpoints (Phase E)
- P2P command group wiring (Phase C)
- `contract abi`, `contract estimate-gas`, and full `contract` CLI group
- `admin rotate-consensus-key`, `admin tls-generate`, and full `admin` CLI group

## Gap Closure Progress

| Phase | Task | Status | Date |
|-------|------|--------|------|
| 0 | Compilation fix (gas metering) | ✅ Complete | 2026-05-19 |
| 0A | Architecture fixes (hash→sign, call results, typed fields) | ✅ Complete | 2026-05-19 |
| A | Multi-validator PoA (quorum, round-robin, validator changes) | ✅ Complete | 2026-05-19 |
| B | Rollback logs / snapshots | ✅ Complete | 2026-05-19 |
| C | P2P CLI wiring | ✅ Complete | 2026-05-19 |
| D | Contract/Admin CLI (ABI, rotate-key, tls-generate, estimate-gas) | ✅ Complete | 2026-05-20 |
| E | WebSocket API | ✅ Complete | 2026-05-20 |
| F | Spec alignment | ✅ Complete | 2026-05-20 |
| G | Testing (crash recovery, byzantine) | ✅ Complete | 2026-05-20 |
| H | Documentation | ✅ Complete | 2026-05-20 |
| I | SDK/FFI hardening | ❌ Pending | — |
| J | Spec-gap closure (REST, CLI, SDK tests) | ❌ Pending | — |

## Reproducibility

Use the following commands to validate the current compliance snapshot:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
