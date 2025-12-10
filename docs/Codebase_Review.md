# Codebase Review Summary

Scope: read all documents in `docs/` (overview, runtime, consensus, ledger/state, storage, mempool, smart contracts, CLI/SDK) and reviewed the current Rust implementation under `src/`.

Key gaps and risks
- **Consensus stubs** (`src/consensus.rs`): `validate_block` is a no-op and `sign_block` drops the signature, so PoA authority is never enforced. Block timestamps are just `previous + 1`, ignoring wall-clock limits and the configured interval.
- **Chain initialization/storage** (`src/ledger.rs` + `src/storage.rs`): `Ledger::initialize_chain` writes via `apply_batch`, but `SledStorage::apply_batch` applies to the default DB tree, while reads use dedicated trees (e.g., `chain_state_tree`). As a result, genesis/chain state are not retrievable through the normal getters, so `Runtime::new` can fail with `ChainInitializationError`.
- **Transaction creation path** (`src/main.rs`): CLI commands build transactions with zeroed hashes, nonces, gas, and signatures; they never call `Transaction::sign` or submit to the runtime, so produced transactions are not valid or chain-aware.
- **State transition issues** (`src/ledger.rs`): Contract deploy overwrites the sender account with a contract account instead of creating a separate contract entry; contract calls ignore execution results/errors; nonce handling assumes pre-existing accounts (runtime earlier defaults to creating a zero-balance account but ledger requires it to exist).
- **Contract engine stubs** (`src/contracts.rs`): `call_contract`/`query_contract` return empty results and skip WASM execution; deploy stores WASM but skips validation and init logic.
- **Sync/mempool**: Runtime uses an in-memory `Vec` mempool (not the sled-backed mempool), so pending transactions are lost across runs and unrelated to the storage mempool API. Sync layer is a partial handshake only; `sync_with_peer` always errors.

Suggested improvements (prioritized)
1) Fix `SledStorage::apply_batch` to route writes through the correct trees (blocks, chain_state, accounts, tx index) or replace with a sled transaction; then re-run `Ledger::initialize_chain` to persist genesis state.
2) Enforce PoA signing/validation: attach a block signature to metadata, verify against `authorized_signer_key`, and use real timestamps with tolerance checks.
3) Make CLI transactions chain-aware: fetch nonce from storage, set gas limits, compute hash via `Transaction::calculate_hash`, sign with the provided key, and submit through `Runtime::submit_transaction`.
4) Adjust ledger state transitions: on contract deploy, create a new contract account without clobbering the sender; propagate contract execution results/errors; gracefully create missing wallet accounts when allowed by policy.
5) Implement minimal WASM execution or clearly gate the stubbed contract engine behind a feature flag to avoid implying production readiness.
6) Align runtime mempool with storage-backed mempool (or document in-memory behavior) and add simple eviction/ordering to prevent unbounded growth.

Tests: none run (documentation-only change). Recommend adding integration tests around genesis initialization, block production, and transaction signing once storage/apply_batch is corrected.
